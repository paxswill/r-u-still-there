// SPDX-License-Identifier: GPL-3.0-or-later
use futures::future::{FutureExt, TryFutureExt};
use futures::sink::{self, SinkExt};
use futures::stream::{FuturesUnordered, Stream, StreamExt, TryStream};
use http::Response;
use linux_embedded_hal::I2cdev;
use ndarray::Array2;
use thermal_camera::grideye;
use tokio::time::Duration;
use warp::Filter;

use std::convert::TryFrom;
use std::fs;
use std::path::Path;
use std::vec::Vec;

#[macro_use]
extern crate lazy_static;

mod camera;
mod error;
mod image_buffer;
mod render;
mod settings;
mod spmc;
mod stream;

use crate::render::Renderer as _;
use crate::settings::{CameraOptions, CameraSettings, I2cSettings, Settings, StreamSettings};
use crate::stream::VideoStream;

fn ok_stream<T, St, E>(in_stream: St) -> impl TryStream<Ok = T, Error = E, Item = Result<T, E>>
where
    St: Stream<Item = T>,
{
    in_stream.map(Result::<T, E>::Ok)
}

#[derive(Debug, Default)]
struct App {
    frame_source: Option<spmc::Sender<Array2<f32>>>,
    rendered_source: Option<spmc::Sender<image_buffer::ImageBuffer>>,
    tasks: FuturesUnordered<tokio::task::JoinHandle<Result<(), error::Error<bytes::Bytes>>>>,
}

impl App {
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
        let frame_stream = camera::camera_stream(camera_device, Duration::from(common_options));
        let frame_multiplexer = spmc::Sender::default();
        let frame_future = ok_stream(frame_stream).forward(frame_multiplexer.clone());
        self.frame_source = Some(frame_multiplexer);
        self.tasks.push(tokio::spawn(frame_future.err_into()));
    }

    // TODO: once the render config settings is set up, have this function take that as an
    // argument. For now it just creates the hardcoded values.
    fn create_renderer(&mut self) -> Result<(), &str> {
        let renderer = render::SvgRenderer::new(
            render::Limit::Static(15.0),
            render::Limit::Static(30.0),
            render::TemperatureDisplay::Celsius,
            50,
            colorous::TURBO,
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
        self.tasks.push(tokio::spawn(render_future.err_into()));
        Ok(())
    }

    fn create_streams(&mut self, settings: StreamSettings) -> Result<(), &str> {
        // Bail out if there aren't any stream sources enabled
        // For now there's just MJPEG, but HLS is planned for the future.
        if !settings.mjpeg {
            // It's OK, there was just nothing to do.
            return Ok(());
        }

        let mut routes = Vec::new();

        if settings.mjpeg {
            // MJPEG "sink"
            let mjpeg = stream::mjpeg::MjpegStream::new();
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
            let mjpeg_sink = sink::unfold(
                mjpeg,
                |mut mjpeg, frame: image_buffer::ImageBuffer| async move {
                    mjpeg.send_frame(&frame)?;
                    Ok::<_, stream::mjpeg::FrameError>(mjpeg)
                },
            );
            let rendered_stream = self
                .rendered_source
                .as_ref()
                // TODO: also Error here
                .ok_or("need to create renderer first")?
                .stream();
            let mjpeg_future = ok_stream(rendered_stream).forward(mjpeg_sink);
            self.tasks
                .push(tokio::spawn(mjpeg_future.err_into::<error::Error<_>>()));
        }
        let combined_route = routes
            .into_iter()
            .reduce(|combined, next| combined.or(next).unify().boxed())
            // TODO: more error-ing
            .ok_or("problem creating streaming routes")?;
        self.tasks.push(tokio::spawn(
            warp::serve(combined_route)
                .bind(settings)
                .map(Ok),
        ));
        Ok(())
    }

    fn tasks(
        &mut self,
    ) -> &mut FuturesUnordered<tokio::task::JoinHandle<Result<(), error::Error<bytes::Bytes>>>>
    {
        &mut self.tasks
    }
}

impl<'a> From<Settings<'a>> for App {
    fn from(config: Settings<'a>) -> Self {
        let mut app = Self::default();
        app.create_camera(config.camera);
        app.create_renderer().unwrap();
        app.create_streams(config.streams).unwrap();
        app
    }
}

#[tokio::main]
async fn main() {
    // Static config location (and relative!) for now
    let config_data = fs::read(Path::new("./config.toml")).unwrap();
    let config: Settings = toml::from_slice(&config_data).unwrap();

    let mut app = App::from(config);

    let mut ok_all = ok_stream(app.tasks());
    let mut drain = sink::drain();

    tokio::join!(drain.send_all(&mut ok_all)).0.unwrap();
}
