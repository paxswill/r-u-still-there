use futures::stream::StreamExt;
use http::Response;
use linux_embedded_hal::I2cdev;
use thermal_camera::grideye;
use tokio::time::Duration;
use warp::Filter;

use std::path::Path;

#[macro_use]
extern crate lazy_static;

mod camera;
mod image_buffer;
mod render;
mod stream;

use crate::stream::{MjpegStream, VideoStream};

#[tokio::main]
async fn main() {
    // MJPEG "sink"
    let mjpeg = MjpegStream::new();

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
    let renderer = render::Renderer::new(
        render::Limit::Static(15.0),
        render::Limit::Static(30.0),
        render::TemperatureDisplay::Celsius,
        50,
        colorous::TURBO,
    );
    let rendered_frames =
        frame_stream.map(move |temperatures| renderer.render_buffer(&temperatures));

    // Stream them out via MJPEG
    let mjpeg_stream = rendered_frames.for_each(move |pixel_buf| {
        let mut mjpeg = mjpeg.clone();
        futures::future::lazy(move |_| {
            mjpeg.send_frame(pixel_buf.as_ref()).unwrap();
        })
    });

    tokio::join!(
        //tokio::spawn(warp::serve(mjpeg).bind(([127, 0, 0, 1], 9000))),
        tokio::spawn(warp::serve(mjpeg_route).bind(([0, 0, 0, 0], 9000))),
        mjpeg_stream
    )
    .0
    .unwrap();
}
