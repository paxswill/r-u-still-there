// SPDX-License-Identifier: GPL-3.0-or-later
use futures::future::{Future, FutureExt, TryFutureExt};
use futures::stream::{FuturesUnordered, Stream, StreamExt, TryStream};
use http::Response;
use image::flat::{FlatSamples, SampleLayout};
use image::imageops;
use linux_embedded_hal::{I2cdev, i2cdev::linux::LinuxI2CError};
use thermal_camera::{grideye, ThermalCamera};
use tokio::task::JoinError;
use tokio::time::{self, Duration};
use tokio_stream::wrappers::IntervalStream;
use tracing::{debug, info, info_span, instrument, trace_span};
use tracing_futures::Instrument;
use warp::Filter;

use std::convert::TryFrom;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::vec::Vec;

use crate::image_buffer::{BytesImage, ThermalImage};
use crate::occupancy::Tracker;
use crate::render::Renderer as _;
use crate::settings::{
    CameraSettings, CommonOptions, I2cSettings, RenderSettings, Rotation, Settings, StreamSettings,
    TrackerSettings,
};
use crate::{error, render, spmc, stream};

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

pub struct Pipeline {
    camera: Arc<Mutex<dyn ThermalCamera<Error = LinuxI2CError> + Send + Sync>>,
    frame_source: Option<spmc::Sender<ThermalImage>>,
    rendered_source: Option<spmc::Sender<BytesImage>>,
    tasks: TaskList,
}

impl Pipeline {
    #[instrument]
    fn create_camera(
        settings: &CameraSettings,
    ) -> Arc<Mutex<dyn ThermalCamera<Error = LinuxI2CError> + Send + Sync>> {
        let i2c_config: &I2cSettings = settings.into();
        // TODO: Add From<I2cError>
        let bus = I2cdev::try_from(i2c_config).unwrap();
        // TODO: Add From<I2cError>
        let addr = grideye::Address::try_from(i2c_config.address).unwrap();
        // TODO: Move this into a TryFrom implementation or something on CameraSettings
        Arc::new(Mutex::new(match settings {
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
        }))
    }

    fn wrap_camera_stream(
        camera: &Arc<Mutex<dyn ThermalCamera<Error = LinuxI2CError> + Send + Sync>>,
        common_options: CommonOptions,
    ) -> impl Stream<Item = ThermalImage> {
        let interval = time::interval(Duration::from(common_options));
        let interval_stream = IntervalStream::new(interval);
        let camera = Arc::clone(camera);
        interval_stream
            .map(move |_| camera.lock().unwrap().image().unwrap())
            .map(move |array2| {
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
                let mut temperatures = buffer_image.try_into_buffer().unwrap();
                // ThermalCamera has the origin in the lower left, while image has it in the upper
                // left. Flip the image vertically by default to compensate for this, but skip the
                // flip if the user wants it flipped.
                if !common_options.flip_vertical {
                    imageops::flip_vertical_in_place(&mut temperatures);
                }
                // The rest of the basic image transformations
                if common_options.flip_horizontal {
                    imageops::flip_horizontal_in_place(&mut temperatures);
                }
                temperatures = match common_options.rotation {
                    Rotation::Zero => temperatures,
                    Rotation::Ninety => imageops::rotate90(&temperatures),
                    Rotation::OneEighty => {
                        imageops::rotate180_in_place(&mut temperatures);
                        temperatures
                    }
                    Rotation::TwoSeventy => imageops::rotate270(&temperatures),
                };
                temperatures
            })
            .instrument(info_span!("frame_stream"))
            .boxed()
    }

    fn configure_camera(&mut self, settings: CameraSettings) {
        let frame_stream = Self::wrap_camera_stream(&self.camera, CommonOptions::from(settings));
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
            .instrument(trace_span!("render_stream"))
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
            info!("video streams disabled, skipping setup");
            // It's Ok, there was just nothing to do.
            return Ok(());
        }
        let mut routes = Vec::new();
        if settings.mjpeg {
            debug!("creating MJPEG encoder");
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
            self.tasks.push(Box::new(
                tokio::spawn(mjpeg.instrument(trace_span!("mjpeg_encoder"))).err_into(),
            ));
        }
        let combined_route = routes
            .into_iter()
            .reduce(|combined, next| combined.or(next).unify().boxed())
            // TODO: more error-ing
            .ok_or("problem creating streaming routes")?;
        let bind_address: std::net::SocketAddr = settings.into();
        debug!(address = ?bind_address, "creating warp server");
        let server = warp::serve(combined_route).bind(bind_address);
        self.tasks.push(Box::new(
            server.instrument(info_span!("warp_server")).map(Ok),
        ));
        Ok(())
    }

    fn create_tracker(&mut self, settings: TrackerSettings) -> Result<(), &str> {
        let tracker = Tracker::from(&settings);
        let logged_count_stream = tracker
            .count_stream()
            .instrument(info_span!("occupancy_count_stream"))
            .inspect(|count| {
                info!(occupancy_count = count, "occupancy count changed");
            });
        let frame_stream = self
            .frame_source
            .as_ref()
            .ok_or("need to create frame source first")?
            .stream();
        self.tasks.push(Box::new(
            ok_stream(logged_count_stream)
                .forward(futures::sink::drain())
                .err_into(),
        ));
        self.tasks.push(Box::new(
            ok_stream(frame_stream).forward(tracker).err_into(),
        ));
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

impl From<Settings> for Pipeline {
    fn from(config: Settings) -> Self {
        let camera = Self::create_camera(&config.camera);
        let mut app = Self {
            camera,
            frame_source: None,
            rendered_source: None,
            tasks: TaskList::default(),
        };
        app.configure_camera(config.camera);
        app.create_renderer(config.render).unwrap();
        app.create_streams(config.streams).unwrap();
        app.create_tracker(config.tracker).unwrap();
        app
    }
}
