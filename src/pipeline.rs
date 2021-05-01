// SPDX-License-Identifier: GPL-3.0-or-later
use futures::future::{Future, FutureExt, TryFutureExt};
use futures::stream::{FuturesUnordered, Stream, StreamExt, TryStream};
use http::Response;
use image::flat::{FlatSamples, SampleLayout};
use linux_embedded_hal::I2cdev;
use thermal_camera::grideye;
use tokio::task::JoinError;
use tokio::time::Duration;
use warp::Filter;

use std::convert::TryFrom;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::vec::Vec;

use crate::image_buffer::{BytesImage, ThermalImage};
use crate::render::Renderer as _;
use crate::settings::{
    CameraOptions, CameraSettings, I2cSettings, RenderSettings, Settings, StreamSettings,
};
use crate::{camera, error, render, spmc, stream};

fn ok_stream<T, St, E>(in_stream: St) -> impl TryStream<Ok = T, Error = E, Item = Result<T, E>>
where
    St: Stream<Item = T>,
{
    in_stream.map(Result::<T, E>::Ok)
}

type InnerResult = Result<(), error::Error<bytes::Bytes>>;
type TaskList = FuturesUnordered<Box<dyn Future<Output = InnerResult> + Unpin>>;

fn flatten_join_result<E>(join_result: Result<Result<(), E>, JoinError>) -> InnerResult
where
    error::Error<bytes::Bytes>: From<E>,
{
    match join_result {
        Ok(inner_result) => Ok(inner_result?),
        Err(join_error) => {
            if join_error.is_panic() {
                join_error.into_panic();
                unreachable!()
            } else {
                Err(error::Error::CancelledThread(join_error))
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Pipeline {
    frame_source: Option<spmc::Sender<ThermalImage>>,
    rendered_source: Option<spmc::Sender<BytesImage>>,
    tasks: TaskList,
}

impl Pipeline {
    fn create_camera(&mut self, settings: CameraSettings) {
        let common_options = CameraOptions::from(settings);
        let i2c_config = I2cSettings::from(settings);
        // TODO: Add From<I2cError>
        let bus = I2cdev::try_from(i2c_config).unwrap();
        // TODO: Add From<I2cError>
        let addr = grideye::Address::try_from(i2c_config.address).unwrap();
        // TODO: Move this into a TryFrom implementation or something on CameraSettings
        let camera_device = match settings {
            CameraSettings::GridEye {
                i2c: _,
                options: common_options,
            } => {
                let mut cam = grideye::GridEye::new(bus, addr);
                cam.set_frame_rate(match common_options.frame_rate {
                    2..=10 => grideye::FrameRateValue::Fps10,
                    1 => grideye::FrameRateValue::Fps1,
                    // The config deserializing validates the given frame rate against the camera type.
                    _ => unreachable!(),
                })
                .unwrap();
                cam
            }
        };
        let frame_stream = camera::camera_stream(camera_device, Duration::from(common_options))
            .map(|array2| {
                let (row_count, col_count) = array2.dim();
                let height = row_count as u32;
                let width = col_count as u32;
                // Force the layout to row-major. If it's already in that order, this is a noop
                // (and it *should* be in row-major order already).
                let array2 = if array2.is_standard_layout() {
                    array2
                } else {
                    array2.reversed_axes()
                };
                let layout = SampleLayout::row_major_packed(1, width, height);
                let buffer_image = FlatSamples {
                    samples: array2.into_raw_vec(),
                    layout,
                    color_hint: None,
                };
                // The provided ndarray is in standard layout, meaning all its data is contiguous.
                // The preconditions for try_into_buffer should all be met, so panic if there's a
                // problem.
                buffer_image.try_into_buffer().unwrap()
            });
        let frame_multiplexer = spmc::Sender::default();
        let frame_future = ok_stream(frame_stream).forward(frame_multiplexer.clone());
        self.frame_source = Some(frame_multiplexer);
        self.tasks.push(Box::new(frame_future.err_into()));
    }

    fn create_renderer(&mut self, settings: RenderSettings) -> Result<(), &str> {
        let renderer = render::SvgRenderer::new(
            settings.lower_limit,
            settings.upper_limit,
            render::TemperatureDisplay::from(settings.units),
            settings.grid_size,
            settings.colors,
        );
        let rendered_stream = self
            .frame_source
            .as_ref()
            // TODO: use a real Error here
            .ok_or("need to create a frame stream first")?
            .stream()
            .map(move |temperatures| renderer.render_buffer(&temperatures));
        let rendered_multiplexer = spmc::Sender::default();
        let render_future = ok_stream(rendered_stream).forward(rendered_multiplexer.clone());
        self.rendered_source = Some(rendered_multiplexer);
        self.tasks.push(Box::new(
            tokio::spawn(render_future).map(flatten_join_result),
        ));
        Ok(())
    }

    fn create_streams(&mut self, settings: StreamSettings) -> Result<(), &str> {
        // Bail out if there aren't any stream sources enabled.
        // For now there's just MJPEG, but HLS is planned for the future.
        if !settings.mjpeg {
            // It's Ok, there was just nothing to do.
            return Ok(());
        }
        let mut routes = Vec::new();
        if settings.mjpeg {
            // MJPEG sink
            let render_source = self
                .rendered_source
                .as_ref()
                // TODO: also Error here
                .ok_or("need to create renderer first")?;
            let mjpeg = stream::mjpeg::MjpegStream::new(render_source);
            let mjpeg_output = mjpeg.clone();
            let mjpeg_route = warp::path("mjpeg")
                .and(warp::path::end())
                .map(move || {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", mjpeg_output.content_type())
                        .body(mjpeg_output.body())
                })
                .boxed();
            routes.push(mjpeg_route);

            // Stream out rendered frames via MJPEG
            self.tasks.push(Box::new(tokio::spawn(mjpeg).err_into()));
        }
        let combined_route = routes
            .into_iter()
            .reduce(|combined, next| combined.or(next).unify().boxed())
            // TODO: more error-ing
            .ok_or("problem creating streaming routes")?;
        self.tasks
            .push(Box::new(warp::serve(combined_route).bind(settings).map(Ok)));
        Ok(())
    }
}

impl Future for Pipeline {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.tasks.poll_next_unpin(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(option) => match option {
                    None => return Poll::Ready(()),
                    Some(_) => (),
                },
            }
        }
    }
}

impl<'a> From<Settings<'a>> for Pipeline {
    fn from(config: Settings<'a>) -> Self {
        let mut app = Self::default();
        app.create_camera(config.camera);
        app.create_renderer(config.render).unwrap();
        app.create_streams(config.streams).unwrap();
        app
    }
}
