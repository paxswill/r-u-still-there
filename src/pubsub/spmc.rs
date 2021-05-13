// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;

use futures::task::{AtomicWaker, Context, Poll};
use futures::{Sink, Stream};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Debug)]
pub struct Sender<T> {
    inner: broadcast::Sender<T>,
    waker: Arc<AtomicWaker>,
}

impl<T: 'static + Clone + Send> Sender<T> {
    pub fn new(capacity: usize) -> Self {
        let (inner, _) = broadcast::channel(capacity);
        Self {
            inner,
            waker: Arc::new(AtomicWaker::new()),
        }
    }

    pub fn stream(&self) -> impl Stream<Item = T> {
        // Wake any wakers up if needed
        self.waker.wake();
        BroadcastStream::new(self.inner.subscribe()).filter_map(Result::ok)
    }
}

impl<T: 'static + Clone + Send> Sink<T> for Sender<T> {
    type Error = Infallible;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Pending if there's no receivers, always ready otherwise.
        // Skip registration if we can
        if self.inner.receiver_count() != 0 {
            return Poll::Ready(Ok(()));
        }
        // Register, then check again in case a subscriber was added between the first check and
        // the registration.
        self.waker.register(cx.waker());
        if self.inner.receiver_count() == 0 {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        // Send errors are transient, and we don't care about how many receviers there are.
        let _ = self.inner.send(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Always done flushing
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Always done flushing
        Poll::Ready(Ok(()))
    }
}

impl<T: 'static + Clone + Send> Default for Sender<T> {
    fn default() -> Self {
        Self::new(1)
    }
}
