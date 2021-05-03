// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::{Buf, Bytes};
use futures::sink::{Sink, SinkExt};
use futures::stream::{Stream, StreamExt};
use futures::{ready, Future};
use hyper::Body;

#[cfg(feature = "mozjpeg")]
use mozjpeg::{ColorSpace, Compress};

use bytes::{BufMut, BytesMut};
use image::codecs::jpeg::JpegEncoder;

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::image_buffer::BytesImage;
use crate::spmc::Sender;

type StreamBox = Box<dyn Stream<Item = BytesImage> + Send + Sync + Unpin>;
type OptionalStreamBox = Arc<Mutex<Option<StreamBox>>>;

#[derive(Clone)]
pub struct MjpegStream {
    boundary: String,
    sender: Sender<Bytes>,
    render_source: Sender<BytesImage>,
    render_stream: OptionalStreamBox,
    temp_image: Option<BytesImage>,
}

impl MjpegStream {
    pub fn new(render_source: &Sender<BytesImage>) -> Self {
        Self {
            // TODO: randomize boundary
            boundary: "mjpeg_rs_boundary".to_string(),
            sender: Sender::default(),
            render_source: render_source.clone(),
            render_stream: Arc::new(Mutex::new(None)),
            temp_image: None,
        }
    }

    pub fn body(&self) -> Body {
        // The only kind of error BroadcastStream can send is when a receiver is lagged. In that
        // case just continue reading as the next recv() will work.
        let jpeg_stream = self.sender.stream();
        let result_stream = jpeg_stream.map(Result::<Bytes, hyper::http::Error>::Ok);
        Body::wrap_stream(result_stream)
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

impl Future for MjpegStream {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.sender.poll_ready_unpin(cx) {
                Poll::Pending => {
                    // Assume that poll_ready_unpin takes care of registering a waker
                    // Drop the render_stream as there's nowhere for it to go.
                    self.render_stream.lock().unwrap().take();
                    return Poll::Pending;
                }
                Poll::Ready(Ok(_)) => {
                    if let Some(image) = self.temp_image.take() {
                        self.send_image(image).unwrap();
                    }
                    self.temp_image = {
                        let mut stream_option = self.render_stream.lock().unwrap();
                        let stream = stream_option
                            .get_or_insert_with(|| Box::new(self.render_source.stream()));
                        ready!(stream.poll_next_unpin(cx))
                    };
                    match self.temp_image {
                        None => return Poll::Ready(()),
                        Some(_) => (),
                    }
                }
                // spmc::Sender's error is Infallible.
                Poll::Ready(Err(_)) => unreachable!(),
            }
        }
    }
}

impl Sink<BytesImage> for MjpegStream {
    type Error = Infallible;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
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