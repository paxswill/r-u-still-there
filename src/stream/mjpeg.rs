use bytes::{Buf, Bytes};
use futures::stream::StreamExt;
use hyper::Body;
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;

#[cfg(feature = "mozjpeg")]
use mozjpeg::{ColorSpace, Compress};

#[cfg(feature = "image")]
use bytes::{BufMut, BytesMut};
#[cfg(feature = "image")]
use image::{codecs::jpeg::JpegEncoder, ColorType};

use std::sync::{Arc, Mutex};

use crate::image_buffer::ImageBuffer;
use crate::stream::VideoStream;

type WriteChannel = watch::Sender<Bytes>;
type ReadChannel = watch::Receiver<Bytes>;

// SPDX-License-Identifier: GPL-3.0-or-later
#[derive(Clone, Debug)]
pub struct MjpegStream {
    boundary: String,
    rx_handle: ReadChannel,
    tx_handle: Arc<Mutex<WriteChannel>>,
}

impl MjpegStream {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(Bytes::default());
        Self {
            // TODO: randomize boundary
            boundary: "mjpeg_rs_boundary".to_string(),
            rx_handle: rx,
            tx_handle: Arc::new(Mutex::new(tx)),
        }
    }

    pub fn body(&self) -> Body {
        let jpeg_stream = WatchStream::new(self.rx_handle.clone());
        let result_stream = jpeg_stream.map(Result::<Bytes, hyper::http::Error>::Ok);
        Body::wrap_stream(result_stream)
    }

    pub fn content_type(&self) -> String {
        format!("multipart/x-mixed-replace; boundary={}", self.boundary)
    }
}

type FrameError = watch::error::SendError<Bytes>;

impl VideoStream<FrameError> for MjpegStream {
    fn send_frame(&mut self, buf: &dyn ImageBuffer) -> Result<(), FrameError> {
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
        self.tx_handle.lock().unwrap().send(owned)
    }
}

#[cfg(feature = "mozjpeg")]
fn encode_jpeg_mozjpeg(image: &dyn ImageBuffer) -> Bytes {
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
fn encode_jpeg_image(image: &dyn ImageBuffer) -> Bytes {
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
