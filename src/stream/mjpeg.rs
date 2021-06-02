// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::sink::{Sink, SinkExt};
use futures::stream::{Stream, StreamExt};
use futures::{ready, Future};
use hyper::Body;
use image::codecs::jpeg::JpegEncoder;
use pin_project::pin_project;
use tracing::{debug, debug_span, info, trace};

#[cfg(feature = "mozjpeg")]
use mozjpeg::{ColorSpace, Compress};

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::image_buffer::BytesImage;
use crate::spmc::Sender;

type StreamBox = Arc<Mutex<dyn Stream<Item = BytesImage> + Send + Sync + Unpin>>;

#[pin_project]
#[derive(Clone)]
pub struct MjpegStream {
    boundary: String,
    #[pin]
    sender: Sender<Bytes>,
    render_stream: StreamBox,
    temp_image: Option<BytesImage>,
}

impl MjpegStream {
    pub fn new(render_source: &Sender<BytesImage>) -> Self {
        // TODO: randomize boundary
        let boundary = "mjpeg_rs_boundary".to_string();
        debug!(%boundary, "creating new MJPEG encoder");
        Self {
            boundary,
            sender: render_source.new_child(),
            render_stream: Arc::new(Mutex::new(render_source.uncounted_stream())),
            temp_image: None,
        }
    }

    pub fn body(&self) -> Body {
        // The only kind of error BroadcastStream can send is when a receiver is lagged. In that
        // case just continue reading as the next recv() will work.
        let jpeg_stream = self.sender.stream();
        let result_stream = jpeg_stream.map(Result::<Bytes, hyper::http::Error>::Ok);
        info!("creating new MJPEG stream for client");
        Body::wrap_stream(result_stream)
    }

    pub fn content_type(&self) -> String {
        format!("multipart/x-mixed-replace; boundary={}", self.boundary)
    }

    fn send_image(&mut self, buf: BytesImage) -> anyhow::Result<()> {
        let span = debug_span!("send_mjpeg_image");
        let _enter = span.enter();
        let jpeg_buf = encode_jpeg(&buf);
        debug!(jpeg_size = %(jpeg_buf.len()), "encoded image to JPEG");
        let header = Bytes::from(format!(
            "\r\n--{}\r\nContent-Type: image/jpeg\r\n\r\n",
            self.boundary
        ));
        // TODO: this is doing some extra copies.
        let total_length = header.len() + jpeg_buf.len();
        debug!(total_size = total_length, "total frame data length");
        let owned = header.chain(jpeg_buf).copy_to_bytes(total_length);
        // This is alright to call like this, as this method is *only* called from the start_send()
        // method of the Sink trait, and *that* method is required to be called *after*
        // poll_ready(). This types poll_ready() calls self.sender.poll_ready(), ensuring that the
        // sender is ready for the start_send().
        self.sender.start_send_unpin(owned)
    }
}

impl Future for MjpegStream {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match ready!(self.sender.poll_ready_unpin(cx)) {
                Ok(_) => {
                    if let Some(image) = self.temp_image.take() {
                        self.send_image(image).unwrap();
                    }
                    self.temp_image = {
                        let mut stream = self.render_stream.lock().unwrap();
                        ready!(stream.poll_next_unpin(cx))
                    };
                    match self.temp_image {
                        None => return Poll::Ready(()),
                        Some(_) => (),
                    }
                }
                // spmc::Sender's error is Infallible.
                Err(_) => unreachable!(),
            }
        }
    }
}

impl Sink<BytesImage> for MjpegStream {
    type Error = anyhow::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.sender.poll_ready(cx)
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
    trace!("using mozjpeg to encode JPEG image");
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
    trace!("using image crate to encode JPEG image");
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
