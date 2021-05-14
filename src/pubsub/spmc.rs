// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::Infallible;
use std::pin::Pin;

use futures::task::{Context, Poll};
use futures::{ready, Future, Sink, Stream};
use pin_utils::unsafe_pinned;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use super::{CountedStream, TreeCount};

#[derive(Clone, Debug)]
pub struct Sender<T: 'static + Clone + Send> {
    inner: broadcast::Sender<T>,
    count: TreeCount,
}

impl<T: 'static + Clone + Send> Sender<T> {
    unsafe_pinned!(count: TreeCount);

    pub fn new_child<U>(&self) -> Sender<U>
    where
        U: 'static + Clone + Send,
    {
        let (inner, _) = broadcast::channel(1);
        Sender {
            inner,
            count: self.count.new_child(),
        }
    }

    pub fn uncounted_stream(&self) -> impl Stream<Item = T> {
        BroadcastStream::new(self.inner.subscribe()).filter_map(Result::ok)
    }

    pub fn stream(&self) -> impl Stream<Item = T> {
        CountedStream::new(self.count.get_token(), self.uncounted_stream())
    }
}

impl<T: 'static + Clone + Send> Sink<T> for Sender<T> {
    type Error = Infallible;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.count().poll(cx));
        // If we reach here, that means ready!() didn't return early.
        Poll::Ready(Ok(()))
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
        let (inner, _) = broadcast::channel(1);
        Self {
            inner,
            count: TreeCount::default(),
        }
    }
}