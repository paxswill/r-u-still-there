// SPDX-License-Identifier: GPL-3.0-or-later
use async_trait::async_trait;

#[async_trait]
pub trait Publisher {
    type Item;
    type Error;
    async fn publish(&self, item: Self::Item) -> Result<(), Self::Error>;
    fn subscribe(&mut self, subscriber: &dyn Subscriber<Item = Self::Item, Error = Self::Error>);
}

#[async_trait]
pub trait Subscriber {
    type Item;
    type Error;
    async fn receive(&self, item: Self::Item) -> Result<(), Self::Error>;
}