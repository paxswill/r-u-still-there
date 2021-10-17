// SPDX-License-Identifier: GPL-3.0-or-later
//! [`Stream`][futures::Stream] extensions.
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{ready, Stream};
use pin_project::pin_project;

pub trait StreamExt: Stream {
    fn filter_repeated(self) -> FilterRepeated<Self>
    where
        Self: Sized,
        Self::Item: PartialEq + Clone,
    {
        FilterRepeated::new(self)
    }

    fn never_error<E>(self) -> OkStream<Self, E>
    where
        Self: Sized,
    {
        OkStream::new(self)
    }
}

impl<St: Stream> StreamExt for St {}

#[pin_project]
#[derive(Debug)]
pub struct FilterRepeated<St: Stream> {
    #[pin]
    stream: St,
    last_seen: Option<St::Item>,
}

impl<St> FilterRepeated<St>
where
    St: Stream,
    <St as Stream>::Item: PartialEq + Clone,
{
    fn new(stream: St) -> Self {
        Self {
            stream,
            last_seen: None,
        }
    }
}

impl<St> Stream for FilterRepeated<St>
where
    St: Stream,
    <St as Stream>::Item: PartialEq + Clone,
{
    type Item = St::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        Poll::Ready(loop {
            if let Some(next_item) = ready!(this.stream.as_mut().poll_next(cx)) {
                if this.last_seen.as_ref() != Some(&next_item) {
                    *this.last_seen = Some(next_item.clone());
                    break Some(next_item);
                }
            } else {
                break None;
            }
        })
    }
}

#[pin_project]
#[derive(Debug)]
pub struct OkStream<St: Stream, E> {
    #[pin]
    stream: St,
    phantom_error: PhantomData<E>,
}
impl<St: Stream, E> OkStream<St, E> {
    fn new(stream: St) -> Self {
        Self {
            stream,
            phantom_error: PhantomData,
        }
    }
}

impl<St: Stream, E> Stream for OkStream<St, E> {
    type Item = Result<St::Item, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        Poll::Ready(ready!(this.stream.poll_next(cx)).map(Result::<St::Item, E>::Ok))
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use futures::stream::{self, StreamExt as _};

    use super::StreamExt;

    /// Ensure that non-repeated values are passed through [`FilterUnique`].
    #[tokio::test]
    async fn filter_unique_sequence() {
        let s = stream::iter(0..5);
        let v = s.filter_repeated().collect::<Vec<_>>().await;
        assert_eq!(v, (0..5).collect::<Vec<_>>())
    }

    /// Ensure that repeated values are skipped by [`FilterUnique`].
    #[tokio::test]
    async fn filter_unique_doubled_sequence() {
        let s = stream::iter([0, 0, 1, 1, 2, 2, 3, 3, 4, 4]);
        let v = s.filter_repeated().collect::<Vec<_>>().await;
        assert_eq!(v, (0..5).collect::<Vec<_>>())
    }

    /// Ensure that cycling a sequence (with repeats) works correctly.
    #[tokio::test]
    async fn filter_unique_doubled_cycle() {
        let s = stream::iter([0, 0, 1, 1, 0, 0, 1, 1]);
        let v = s.filter_repeated().collect::<Vec<_>>().await;
        assert_eq!(v, vec![0, 1, 0, 1]);
    }

    #[tokio::test]
    async fn never_error() {
        let st = stream::iter(0..5);
        let vec = st
            .never_error()
            .collect::<Vec<Result<_, Infallible>>>()
            .await;
        for (expected, actual) in vec.iter().enumerate() {
            assert!(actual.is_ok());
            assert_eq!(actual.unwrap(), expected);
        }
    }
}
