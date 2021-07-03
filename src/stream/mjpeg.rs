// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::{Buf, Bytes};
use futures::sink::{Sink, SinkExt};
use futures::stream::{Stream, StreamExt};
use futures::{ready, Future};
use hyper::Body;
use pin_project::pin_project;
use tracing::{debug, debug_span, info, trace};

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::spmc::Sender;

type StreamBox = Arc<Mutex<dyn Stream<Item = Bytes> + Send + Sync + Unpin>>;

#[pin_project]
#[derive(Clone)]
pub(crate) struct MjpegStream {
    boundary: String,
    #[pin]
    sender: Sender<Bytes>,
    render_stream: StreamBox,
    temp_image: Option<Bytes>,
}

impl MjpegStream {
    pub(crate) fn new(render_source: &Sender<Bytes>) -> Self {
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

    pub(crate) fn body(&self) -> Body {
        // The only kind of error BroadcastStream can send is when a receiver is lagged. In that
        // case just continue reading as the next recv() will work.
        let jpeg_stream = self.sender.stream();
        let result_stream = jpeg_stream.map(Result::<Bytes, hyper::http::Error>::Ok);
        info!("creating new MJPEG stream for client");
        Body::wrap_stream(result_stream)
    }

    pub(crate) fn content_type(&self) -> String {
        format!("multipart/x-mixed-replace; boundary={}", self.boundary)
    }

    fn send_image(&mut self, jpeg_buf: Bytes) -> anyhow::Result<()> {
        let span = debug_span!("send_mjpeg_image");
        let _enter = span.enter();
        let header = Bytes::from(format!(
            "\r\n--{}\r\nContent-Type: image/jpeg\r\n\r\n",
            self.boundary
        ));
        // TODO: this is doing some extra copies.
        let total_length = header.len() + jpeg_buf.len();
        trace!(total_size = total_length, "total frame data length");
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

impl Sink<Bytes> for MjpegStream {
    type Error = anyhow::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().sender.poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, buf: Bytes) -> Result<(), Self::Error> {
        self.send_image(buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().sender.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().sender.poll_close(cx)
    }
}
