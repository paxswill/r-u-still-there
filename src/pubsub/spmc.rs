// SPDX-License-Identifier: GPL-3.0-or-later
use std::pin::Pin;

use futures::task::{Context, Poll};
use futures::{ready, Future, Sink, Stream};
use pin_project::pin_project;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use super::{CountedStream, TreeCount};

/// A single-producer, multiple consumer channel.
///
/// This channel implements [futures.Sink], but will only accept items when there are consumers.
/// Whether or not there are consumers is tracked using [TreeCount] and [CountedStream].
#[pin_project]
#[derive(Clone, Debug)]
pub struct Sender<T: 'static + Clone + Send> {
    inner: broadcast::Sender<T>,

    #[pin]
    count: TreeCount,
}

impl<T: 'static + Clone + Send> Sender<T> {
    /// Create a new [Sender] with a [TreeCount] set up as a child of this `Sender`'s `TreeCount`.
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

    /// Create a [Stream] that *doesn't* increment the subscriber count.
    ///
    /// This is useful in cases where you have a consumer of a stream that *also* has it's own
    /// [Sender] as a child of this one. The downstream `Sender` will hand out it's own `Stream`,
    /// and in doing so those streams will be counted.
    ///
    /// In other words, this method is useful for building up a pipeline of async "filters".
    pub fn uncounted_stream(&self) -> impl Stream<Item = T> {
        BroadcastStream::new(self.inner.subscribe()).filter_map(Result::ok)
    }

    /// Create a stream that increments the subscriber count.
    ///
    /// When this stream is dropped, the count is automatically decremented.
    pub fn stream(&self) -> impl Stream<Item = T> {
        CountedStream::new(self.count.get_token(), self.uncounted_stream())
    }
}

impl<T: 'static + Clone + Send> Sink<T> for Sender<T> {
    type Error = anyhow::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        ready!(this.count.poll(cx));
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
