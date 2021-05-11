// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::Infallible;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::vec::Vec;

use futures::{Sink, Stream};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Clone, Debug)]
pub struct Sender<T> {
    inner: broadcast::Sender<T>,
    wakers: Arc<Mutex<Vec<Waker>>>,
}

impl<T: 'static + Clone + Send> Sender<T> {
    pub fn new(capacity: usize) -> Self {
        let (inner, _) = broadcast::channel(capacity);
        let wakers = Arc::new(Mutex::new(Vec::default()));
        Self { inner, wakers }
    }

    pub fn stream(&self) -> impl Stream<Item = T> {
        // If there are pending wakers, notify them
        {
            let mut wakers_lock = self.wakers.lock().unwrap();
            let wakers = wakers_lock.deref_mut();
            for waker in wakers.drain(..) {
                waker.wake();
            }
        }
        BroadcastStream::new(self.inner.subscribe()).filter_map(Result::ok)
    }
}

impl<T: 'static + Clone + Send> Sink<T> for Sender<T> {
    type Error = Infallible;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Pending if there's no receivers, always ready otherwise.
        if self.inner.receiver_count() == 0 {
            let mut wakers = self.wakers.lock().unwrap();
            wakers.push(cx.waker().clone());
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
