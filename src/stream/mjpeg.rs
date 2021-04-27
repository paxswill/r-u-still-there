use bytes::{Buf, Bytes};
use futures::stream::StreamExt;
use hyper::Body;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

#[cfg(feature = "mozjpeg")]
use mozjpeg::{ColorSpace, Compress};

#[cfg(feature = "image")]
use bytes::{BufMut, BytesMut};
#[cfg(feature = "image")]
use image::{codecs::jpeg::JpegEncoder, ColorType};

use crate::image_buffer::ImageBuffer;
use crate::stream::VideoStream;

type WriteChannel = broadcast::Sender<Bytes>;

// SPDX-License-Identifier: GPL-3.0-or-later
#[derive(Clone, Debug)]
pub struct MjpegStream {
    boundary: String,
    sender: WriteChannel,
}

impl MjpegStream {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1);
        Self {
            // TODO: randomize boundary
            boundary: "mjpeg_rs_boundary".to_string(),
            sender,
        }
    }

    pub fn body(&self) -> Body {
        // The only kind of error BroadcastStream can send is when a receiver is lagged. In that
        // case just continue reading as the next recv() will work.
        let jpeg_stream =
            BroadcastStream::new(self.sender.subscribe()).filter_map(|result| async move {
                match result {
                    Ok(jpeg) => Some(jpeg),
                    Err(_) => None,
                }
            });
        let result_stream = jpeg_stream.map(Result::<Bytes, hyper::http::Error>::Ok);
        Body::wrap_stream(result_stream)
    }

    pub fn content_type(&self) -> String {
        format!("multipart/x-mixed-replace; boundary={}", self.boundary)
    }
}

pub type FrameError = broadcast::error::SendError<Bytes>;

impl VideoStream<FrameError> for MjpegStream {
    fn send_frame(&mut self, buf: &ImageBuffer) -> Result<usize, FrameError> {
        // Only encode the frame to a jpeg if there's a receiver
        if self.sender.receiver_count() == 0 {
            // Not an error, just nobody listening.
            return Ok(0);
        }
        let jpeg_buf = if cfg!(feature = "mozjpeg") {
            encode_jpeg_mozjpeg(buf)
        } else if cfg!(feature = "image") {
            encode_jpeg_image(buf)
        } else {
            panic!("mjpeg feature enabled, but no jpeg encoders enabled")
        };
        let header = Bytes::from(format!(
            "\r\n--{}\r\nContent-Type: image/jpeg\r\n\r\n",
            self.boundary
        ));
        // TODO: this is doing some extra copies.
        let total_length = header.len() + jpeg_buf.len();
        let owned = header.chain(jpeg_buf).copy_to_bytes(total_length);
        self.sender.send(owned)
    }
}

#[cfg(feature = "mozjpeg")]
fn encode_jpeg_mozjpeg(image: &ImageBuffer) -> Bytes {
    // To make it simpler to use renderers within closures, we're creating a fresh encoder each
    // time this method is called. A little less efficient, but much easier to use.
    let mut jpeg_encoder = Compress::new(ColorSpace::from(image.order()));
    jpeg_encoder.set_color_space(ColorSpace::JCS_RGB);
    // Gotta go fast.
    // Long version: in debug builds the most time is spent in SVG rendering. In
    // optimized release builds though, JPEG encoding using the 'image' crate was taking
    // up the most time. Using mozjpeg/libjpeg-turbo will hopefully drop the CPU usage a
    // bit (and make it possible to do 10 FPS on BeagleBones/RasPi Zeros).
    jpeg_encoder.set_fastest_defaults();
    jpeg_encoder.set_quality(75.0);
    jpeg_encoder.set_mem_dest();
    jpeg_encoder.set_size(image.width(), image.height());
    jpeg_encoder.start_compress();
    // Hope write_scanlines can process everything in one go.
    assert!(jpeg_encoder.write_scanlines(image.data()));
    jpeg_encoder.finish_compress();
    Bytes::from(jpeg_encoder.data_to_vec().unwrap())
}

#[cfg(feature = "image")]
fn encode_jpeg_image(image: &ImageBuffer) -> Bytes {
    let mut jpeg_buf = BytesMut::new().writer();
    let mut encoder = JpegEncoder::new(&mut jpeg_buf);
    encoder
        .encode(
            image.data(),
            image.width() as u32,
            image.height() as u32,
            ColorType::from(image.order()),
        )
        .unwrap();
    jpeg_buf.into_inner().freeze()
}
