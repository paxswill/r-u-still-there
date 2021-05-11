// SPDX-License-Identifier: GPL-3.0-or-later
use futures::executor::block_on;
use tokio::sync::Semaphore;

use std::sync::Arc;

#[derive(Debug)]
// TODO: Should this be Weak instead of Arc ?
pub struct Permit(Arc<Semaphore>);

impl Permit {
    pub fn new(semaphore: Arc<Semaphore>) -> Self {
        Self(semaphore)
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        // Take a semaphore permit and permanently remove it from the permit pool.
        // Because there's no such thing as an async drop, this function will block until it
        // acquires the permit. If there's an error acquiring the permit, that means the semaphore
        // has been closed and we don't care about removing the permit in that case.
        if let Ok(permit) = block_on(self.0.acquire()) {
            permit.forget();
        }
    }
}
