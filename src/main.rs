// SPDX-License-Identifier: GPL-3.0-or-later
use futures::sink;
use futures::stream::{Stream, StreamExt, TryStream};
use http::Response;
use linux_embedded_hal::I2cdev;
use ndarray::Array2;
use thermal_camera::grideye;
use tokio::time::Duration;
use warp::Filter;

use std::convert::TryFrom;
use std::fs;
use std::path::Path;

#[macro_use]
extern crate lazy_static;

mod camera;
mod image_buffer;
mod render;
mod settings;
mod spmc;
mod stream;

use crate::render::Renderer as _;
use crate::settings::{Settings, CameraOptions, CameraSettings, I2cSettings};
use crate::stream::VideoStream;

fn ok_stream<T, St, E>(in_stream: St) -> impl TryStream<Ok = T, Error = E, Item = Result<T, E>>
where
    St: Stream<Item = T>,
    //TSt: TryStream<Ok = T, Error = Infallible>
{
    in_stream.map(Result::<T, E>::Ok)
}

#[tokio::main]
async fn main() {
    // Static config location (and relative!) for now
    let config_data = fs::read(Path::new("./config.toml")).unwrap();
    let config: Settings = toml::from_slice(&config_data).unwrap();

    // Temperature grid "source"
    let camera_config = config.camera;
    let common_options = CameraOptions::from(camera_config);
    let i2c_config = I2cSettings::from(camera_config);
    let bus = I2cdev::try_from(i2c_config).unwrap();
    let addr = grideye::Address::try_from(i2c_config.address).unwrap();
    // TODO: Move this into a TryFrom implementation or something on CameraSettings
    let camera_device = match camera_config {
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
    let frame_multiplexer = spmc::Sender::<Array2<f32>>::default();
    let frame_future = ok_stream(frame_stream).forward(frame_multiplexer.clone());

    // Rendering "filter"
    let renderer = render::SvgRenderer::new(
        render::Limit::Static(15.0),
        render::Limit::Static(30.0),
        render::TemperatureDisplay::Celsius,
        50,
        colorous::TURBO,
    );
    let rendered_stream = frame_multiplexer
        .stream()
        .map(move |temperatures| renderer.render_buffer(&temperatures));
    let rendered_multiplexer = spmc::Sender::default();
    let render_future = ok_stream(rendered_stream).forward(rendered_multiplexer.clone());

    // MJPEG "sink"
    let mjpeg = stream::mjpeg::MjpegStream::new();
    let mjpeg_output = mjpeg.clone();
    let mjpeg_route = warp::path("stream").map(move || {
        Response::builder()
            .status(200)
            .header("Content-Type", mjpeg_output.content_type())
            .body(mjpeg_output.body())
    });

    // Stream them out via MJPEG
    let mjpeg_sink = sink::unfold(
        mjpeg,
        |mut mjpeg, frame: image_buffer::ImageBuffer| async move {
            mjpeg.send_frame(&frame)?;
            Ok::<_, stream::mjpeg::FrameError>(mjpeg)
        },
    );
    let mjpeg_future = ok_stream(rendered_multiplexer.stream()).forward(mjpeg_sink);

    tokio::join!(
        tokio::spawn(warp::serve(mjpeg_route).bind(([0, 0, 0, 0], 9000))),
        frame_future,
        render_future,
        mjpeg_future,
    )
    .0
    .unwrap();
}
