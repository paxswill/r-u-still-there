use futures::sink;
use futures::stream::StreamExt;
use http::Response;
use linux_embedded_hal::I2cdev;
use thermal_camera::grideye;
use tokio::time::Duration;
use warp::Filter;

use std::path::Path;

// SPDX-License-Identifier: GPL-3.0-or-later
#[macro_use]
extern crate lazy_static;

mod camera;
mod image_buffer;
mod render;
mod stream;

use crate::render::Renderer as _;
use crate::stream::VideoStream;

#[tokio::main]
async fn main() {
    // MJPEG "sink"
    let mjpeg = stream::mjpeg::MjpegStream::new();
    let mjpeg_output = mjpeg.clone();
    let mjpeg_route = warp::path("stream").map(move || {
        Response::builder()
            .status(200)
            .header("Content-Type", mjpeg_output.content_type())
            .body(mjpeg_output.body())
    });

    // Temperature grid "source"
    let addr = grideye::Address::Secondary;
    // i2c-1 for Raspberry Pi, i2c-2 for Beaglebones.
    let bus_path = Path::new("/dev/i2c-1");
    //let bus_path = Path::new("/dev/i2c-2");
    let bus = I2cdev::new(bus_path).unwrap();
    let frame_stream =
        camera::camera_stream(grideye::GridEye::new(bus, addr), Duration::from_millis(100));

    // Rendering "filter"
    let renderer = render::SvgRenderer::new(
        render::Limit::Static(15.0),
        render::Limit::Static(30.0),
        render::TemperatureDisplay::Celsius,
        50,
        colorous::TURBO,
    );

    let rendered_frames =
        frame_stream.map(move |temperatures| renderer.render_buffer(&temperatures));

    // Stream them out via MJPEG
    let mjpeg_sink = sink::unfold(
        mjpeg,
        |mut mjpeg, frame: Box<dyn image_buffer::ImageBuffer>| async move {
            mjpeg.send_frame(frame.as_ref())?;
            Ok::<_, stream::mjpeg::FrameError>(mjpeg)
        },
    );
    // StreamExt::forward needs a TryStream, which we get by wrapping rendered frames in a
    // Result::Ok.
    let video_future = rendered_frames.map(Ok).forward(mjpeg_sink);

    tokio::join!(
        //tokio::spawn(warp::serve(mjpeg).bind(([127, 0, 0, 1], 9000))),
        tokio::spawn(warp::serve(mjpeg_route).bind(([0, 0, 0, 0], 9000))),
        video_future
    )
    .0
    .unwrap();
}
