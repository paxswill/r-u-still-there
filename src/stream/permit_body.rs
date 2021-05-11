// SPDX-License-Identifier: GPL-3.0-or-later
use hyper::body::HttpBody;
use hyper::header::{HeaderMap, HeaderValue};
use pin_utils::unsafe_pinned;
use tokio::sync::Semaphore;

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::pubsub::Permit;

#[derive(Debug)]
pub struct PermitBody<B: HttpBody> {
    body: B,
    temporary_permits: Vec<Permit>,
}

impl<B: HttpBody> PermitBody<B> {
    unsafe_pinned!(body: B);

    pub fn add_temp_permit_to(&mut self, semaphore: &Arc<Semaphore>) {
        let semaphore = Arc::clone(semaphore);
        semaphore.add_permits(1);
        self.temporary_permits.push(Permit::new(semaphore));
    }
}

impl<B: HttpBody> From<B> for PermitBody<B> {
    fn from(body: B) -> Self {
        Self {
            body,
            temporary_permits: Vec::new(),
        }
    }
}

impl From<PermitBody<hyper::Body>> for hyper::Body {
    fn from(permit_body: PermitBody<Self>) -> Self {
        permit_body.body
    }
}

impl<B: HttpBody> HttpBody for PermitBody<B> {
    type Data = B::Data;
    type Error = B::Error;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        self.body().poll_data(cx)
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<HeaderMap<HeaderValue>>, Self::Error>> {
        self.body().poll_trailers(cx)
    }
}

impl<B: HttpBody + Unpin> Unpin for PermitBody<B> {}
