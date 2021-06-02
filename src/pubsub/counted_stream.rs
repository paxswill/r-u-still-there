// SPDX-License-Identifier: GPL-3.0-or-later
use futures::Stream;
use pin_project::pin_project;

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::CountToken;

/// A wrapper around a [Stream] that holds a [CountToken] as well.
///
/// By keeping the [CountToken] with the [Steram], a [TreeCount] can be kept in sync.
#[pin_project]
pub struct CountedStream<S: Stream + Send + Sync> {
    token: CountToken,
    #[pin]
    stream: S,
}

impl<S: Stream + Send + Sync> CountedStream<S> {
    pub fn new(token: CountToken, stream: S) -> Self {
        Self { token, stream }
    }
}

impl<S: Stream + Send + Sync> fmt::Debug for CountedStream<S>
where
    S: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("CountedStream")
            .field("token", &self.token)
            .field("stream", &self.stream)
            .finish()
    }
}

impl<S: Stream + Send + Sync> Stream for CountedStream<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.stream.poll_next(cx)
    }
}
