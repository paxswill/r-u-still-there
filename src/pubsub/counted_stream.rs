// SPDX-License-Identifier: GPL-3.0-or-later
use futures::Stream;
use pin_utils::unsafe_pinned;

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::pubsub::CountToken;

/// A wrapper around a [Stream] that holds a [CountToken] as well.
///
/// By keeping the [CountToken] with the [Steram], a [TreeCount] can be kept in sync.
pub struct CountedStream<S: Stream + Send + Sync> {
    token: CountToken,
    stream: S,
}

impl<S: Stream + Send + Sync> CountedStream<S> {
    unsafe_pinned!(stream: S);

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
        self.stream().poll_next(cx)
    }
}

// Conditional Unpin to satisfy safety bounds of unsafe_pinned
impl<S> Unpin for CountedStream<S>
where
    S: Stream + Send + Sync,
    S: Unpin,
{
}
