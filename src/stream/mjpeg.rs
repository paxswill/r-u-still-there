// SPDX-License-Identifier: GPL-3.0-or-later
use async_trait::async_trait;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::ready;
use futures::sink::{Sink, SinkExt};
use futures::stream::StreamExt;
use hyper::body::{Body, HttpBody};
use image::codecs::jpeg::JpegEncoder;
use tokio::sync::Semaphore;
use tokio_util::sync::PollSemaphore;

#[cfg(feature = "mozjpeg")]
use mozjpeg::{ColorSpace, Compress};

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use super::permit_body::PermitBody;
use crate::image_buffer::BytesImage;
use crate::spmc::Sender;
use crate::pubsub;

#[derive(Clone, Debug)]
pub struct MjpegStream {
    boundary: String,
    bus: pubsub::Bus<Bytes>,
    temp_image: Option<BytesImage>,
    render_semaphore: PollSemaphore,
    encode_semaphore: PollSemaphore
}

impl MjpegStream {
    pub fn new(signal: &Arc<Semaphore>) -> Self {
        Self {
            // TODO: randomize boundary
            boundary: "mjpeg_rs_boundary".to_string(),
            bus: pubsub::Bus::default(),
            temp_image: None,
            render_semaphore: PollSemaphore::new(Arc::clone(signal)),
            encode_semaphore: PollSemaphore::new(Arc::new(Semaphore::new(0))),
        }
    }

    pub fn body(&self) -> PermitBody<Body>{
        // The only kind of error BroadcastStream can send is when a receiver is lagged. In that
        // case just continue reading as the next recv() will work.
        let jpeg_stream = self.bus.stream();
        let result_stream = jpeg_stream.map(Result::<Bytes, hyper::http::Error>::Ok);
        let body_stream = Body::wrap_stream(result_stream);
        let mut permit_body = PermitBody::from(body_stream);
        permit_body.add_temp_permit_to(&self.render_semaphore.clone_inner());
        permit_body.add_temp_permit_to(&self.encode_semaphore.clone_inner());
        permit_body
    }

    pub fn content_type(&self) -> String {
        format!("multipart/x-mixed-replace; boundary={}", self.boundary)
    }

    fn send_image(&mut self, buf: BytesImage) -> Result<(), Infallible> {
        let jpeg_buf = encode_jpeg(&buf);
        let header = Bytes::from(format!(
            "\r\n--{}\r\nContent-Type: image/jpeg\r\n\r\n",
            self.boundary
        ));
        // TODO: this is doing some extra copies.
        let total_length = header.len() + jpeg_buf.len();
        let owned = header.chain(jpeg_buf).copy_to_bytes(total_length);
        self.sender.start_send_unpin(owned)
    }
}

#[async_trait]
impl pubsub::Subscriber for MjpegStream {
    type Item = BytesImage;
    type Error = Infallible;

    async fn receive(&self, item: Self::Item) -> Result<(), Self::Error> {
        // If there are permits available (meaning there are streaming clients connected), encode
        // an image and send it out.
        ready!(self.encode_semaphore.poll_acquire(cx));
    }
}

impl Sink<BytesImage> for MjpegStream {
    type Error = Infallible;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // We're only ready if there are active clients, which is signified by there being
        // available permits in the semaphore.
        ready!(self.encode_semaphore.poll_acquire(cx));
        // Check if the sender is ready
        self.sender.poll_ready_unpin(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, buf: BytesImage) -> Result<(), Self::Error> {
        self.send_image(buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.sender.poll_flush_unpin(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.sender.poll_close_unpin(cx)
    }
}

#[cfg(feature = "mozjpeg")]
fn encode_jpeg_mozjpeg(image: &BytesImage) -> Bytes {
    // To make it simpler to use renderers within closures, we're creating a fresh encoder each
    // time this method is called. A little less efficient, but much easier to use.
    // BytesImage is defined to be RGBA.
    let mut jpeg_encoder = Compress::new(ColorSpace::JCS_EXT_RGBA);
    jpeg_encoder.set_color_space(ColorSpace::JCS_RGB);
    // Gotta go fast.
    // Long version: in debug builds the most time is spent in SVG rendering. In
    // optimized release builds though, JPEG encoding using the 'image' crate was taking
    // up the most time. Using mozjpeg/libjpeg-turbo will hopefully drop the CPU usage a
    // bit (and make it possible to do 10 FPS on BeagleBones/RasPi Zeros).
    jpeg_encoder.set_fastest_defaults();
    jpeg_encoder.set_quality(75.0);
    jpeg_encoder.set_mem_dest();
    jpeg_encoder.set_size(image.width() as usize, image.height() as usize);
    jpeg_encoder.start_compress();
    // Hope write_scanlines can process everything in one go.
    assert!(jpeg_encoder.write_scanlines(image));
    jpeg_encoder.finish_compress();
    Bytes::from(jpeg_encoder.data_to_vec().unwrap())
}

fn encode_jpeg_image(image: &BytesImage) -> Bytes {
    let mut jpeg_buf = BytesMut::new().writer();
    let mut encoder = JpegEncoder::new(&mut jpeg_buf);
    encoder.encode_image(image).unwrap();
    jpeg_buf.into_inner().freeze()
}

#[cfg(not(feature = "mozjpeg"))]
fn encode_jpeg(image: &BytesImage) -> Bytes {
    encode_jpeg_image(image)
}

#[cfg(feature = "mozjpeg")]
fn encode_jpeg(image: &BytesImage) -> Bytes {
    if cfg!(feature = "mozjpeg") {
        encode_jpeg_mozjpeg(image)
    } else {
        encode_jpeg_image(image)
    }
}
