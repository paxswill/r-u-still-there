// SPDX-License-Identifier: GPL-3.0-or-later
use std::fmt;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{ready, Future, Sink};
use rumqttc::{AsyncClient, QoS};
use serde::{Serialize, Serializer};
use tokio::sync::{Mutex, RwLock, RwLockReadGuard};
use tracing::debug;

use super::serialize::serialize;

struct InnerState<T> {
    client: Arc<Mutex<AsyncClient>>,
    value: RwLock<T>,
    topic: String,
    retain: bool,
    qos: QoS,
}

impl<T> InnerState<T>
where
    T: Send + Sync,
    T: Serialize,
{
    async fn publish(&self) -> anyhow::Result<()> {
        let value = SerializeRwLockGuard::from(self.value.read().await);
        let payload_data = serialize(&value)?;
        self.client
            .lock()
            .await
            .publish(&self.topic, self.qos, self.retain, payload_data)
            .await
            .map_err(anyhow::Error::from)
    }
}

impl<T> InnerState<T>
where
    T: Send + Sync,
    T: PartialEq,
{
    pub(crate) async fn update(&self, new_value: T) -> bool {
        let mut current_value = self.value.write().await;
        if *current_value != new_value {
            *current_value = new_value;
            true
        } else {
            false
        }
    }
}

impl<T> InnerState<T>
where
    T: Send + Sync,
    T: fmt::Debug + PartialEq + Serialize,
{
    async fn publish_if_update(&self, value: T) -> anyhow::Result<bool> {
        if self.update(value).await {
            self.publish().await.and(Ok(true))
        } else {
            debug!(value = ?self.value, "Skipping update of unchanged value");
            Ok(false)
        }
    }
}

/// A container for managing MQTT topic state.
pub(crate) struct State<T> {
    inner: Arc<InnerState<T>>,
    in_progress_future: Option<Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + Sync>>>,
}

impl<T> State<T> {
    /// The full topic path for this state.
    pub(crate) fn topic(&self) -> &str {
        &self.inner.topic
    }

}

impl<T> State<T>
where
    T: Send + Sync,
{
    pub(crate) fn new<S>(
        client: Arc<Mutex<AsyncClient>>,
        value: T,
        topic: S,
        retain: bool,
        qos: QoS,
    ) -> Self
    where
        S: Into<String>,
    {
        Self {
            inner: Arc::new(InnerState {
                client,
                value: RwLock::new(value),
                topic: topic.into(),
                retain,
                qos,
            }),
            in_progress_future: None,
        }
    }

    /// Get a reference to the current value
    pub(crate) async fn current<'a>(&'a self) -> SerializeRwLockGuard<'a, T> {
        self.inner.value.read().await.into()
    }
}

impl<T> State<T>
where
    T: Send + Sync,
    T: Default,
{
    pub(crate) fn new_default<S>(client: Arc<Mutex<AsyncClient>>, topic: S) -> Self
    where
        S: Into<String>,
    {
        Self::new(client, T::default(), topic, true, QoS::AtLeastOnce)
    }
}

impl<T> State<T>
where
    T: Send + Sync,
    T: PartialEq,
{
    /// Update the state of this topic, returning whether or not the new value is different than
    /// the old one.
    pub(crate) async fn update(&self, new_value: T) -> bool {
        self.inner.update(new_value).await
    }
}

impl<T> State<T>
where
    T: Send + Sync,
    T: Serialize,
{
    /// Publish the current state.
    pub(crate) async fn publish(&self) -> anyhow::Result<()> {
        self.inner.publish().await
    }
}

impl<T> State<T>
where
    T: Send + Sync,
    T: fmt::Debug + PartialEq + Serialize,
{
    /// Combine [update] and [publish] into a single function.
    pub(crate) async fn publish_if_update(&self, value: T) -> anyhow::Result<bool> {
        self.inner.publish_if_update(value).await
    }
}

impl<T> Clone for State<T>
// Note: no bound on `T: Clone`, as the value is stored within an `Arc`.
{
    /// Clone the [State]. The new [State] can act like a new [Sink] and the old [State] can
    /// continue on as well.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            in_progress_future: None,
        }
    }
}

impl<T> fmt::Debug for State<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("client", &self.inner.client)
            .field("value", &self.inner.value)
            .field("topic", &self.topic())
            .field("retain", &self.inner.retain)
            .field("qos", &self.inner.qos)
            .finish()
    }
}

impl<T> Sink<T> for State<T>
where
    T: 'static,
    T: Clone + fmt::Debug + PartialEq + Serialize,
    T: Send + Sync,
{
    type Error = anyhow::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.poll_flush(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, new_value: T) -> Result<(), Self::Error> {
        assert!(
            self.in_progress_future.is_none(),
            "self.in_progress_future should be None is poll_flush() returned Ready(Ok())"
        );
        let inner = Arc::clone(&self.inner);
        // Capture inner in an async closure
        let in_progress = async move {
            inner.publish_if_update(new_value).await
        };
        self.in_progress_future = Some(Box::pin(in_progress));
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if let Some(fut) = &mut self.in_progress_future {
            match ready!(fut.as_mut().poll(cx)) {
                Err(e) => Poll::Ready(Err(e)),
                Ok(_) => {
                    self.in_progress_future = None;
                    Poll::Ready(Ok(()))
                }
            }
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.poll_flush(cx)
    }
}

/// A newtype wrapper around [RwLockReadGuard] to implement [Serialize].
pub(crate) struct SerializeRwLockGuard<'a, T>(RwLockReadGuard<'a, T>);

impl<'a, T> fmt::Debug for SerializeRwLockGuard<'a, T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a, T> Deref for SerializeRwLockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<'a, T> fmt::Display for SerializeRwLockGuard<'a, T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a, T> From<RwLockReadGuard<'a, T>> for SerializeRwLockGuard<'a, T> {
    fn from(guard: RwLockReadGuard<'a, T>) -> Self {
        Self(guard)
    }
}

impl<'a, T> From<SerializeRwLockGuard<'a, T>> for RwLockReadGuard<'a, T> {
    fn from(wrapped_guard: SerializeRwLockGuard<'a, T>) -> Self {
        wrapped_guard.0
    }
}

impl<'a, T> Serialize for SerializeRwLockGuard<'a, T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}
