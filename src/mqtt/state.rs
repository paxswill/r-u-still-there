// SPDX-License-Identifier: GPL-3.0-or-later
use std::fmt;
use std::sync::Arc;

use bytes::{BufMut, BytesMut};
use rumqttc::{AsyncClient, QoS};
use serde::Serialize;
use tokio::sync::RwLock;
use tracing::debug;

/// A container for managing MQTT topic state.
#[derive(Clone)]
pub(crate) struct State<T> {
    value: Arc<RwLock<T>>,
    topic: String,
    retain: bool,
    qos: QoS,
}

impl<T> State<T> {
    pub(crate) fn new<S>(value: T, topic: S, retain: bool, qos: QoS) -> Self
    where
        S: Into<String>,
    {
        Self {
            value: Arc::new(RwLock::new(value)),
            topic: topic.into(),
            retain,
            qos,
        }
    }

    /// The full topic path for this state.
    pub(crate) fn topic(&self) -> &str {
        &self.topic
    }
}

impl<T> State<T>
where
    T: Clone,
{
    /// Get a copy of the current state of this topic
    pub(crate) async fn current(&self) -> T {
        self.value.read().await.clone()
    }
}

impl<T> State<T>
where
    T: Default,
{
    pub(crate) fn new_default_at<S>(topic: S) -> Self
    where
        S: Into<String>,
    {
        Self::new(T::default(), topic, true, QoS::AtLeastOnce)
    }
}

impl<T> State<T>
where
    T: PartialEq,
{
    /// Update the state of this topic, returning whether or not the new value is different than
    /// the old one.
    pub(crate) async fn update(&mut self, new_value: T) -> bool {
        let mut current_value = self.value.write().await;
        if *current_value != new_value {
            *current_value = new_value;
            true
        } else {
            false
        }
    }
}

impl<T> State<T>
where
    T: Serialize + Clone,
{
    /// Publish the current state using the provided client.
    pub(crate) async fn publish(&self, client: &mut AsyncClient) -> anyhow::Result<()> {
        let mut payload_data = BytesMut::new().writer();
        // TODO figure out how to remove this clone
        let value = self.current().await;
        serde_json::to_writer(&mut payload_data, &value)?;
        client
            .publish_bytes(
                self.topic(),
                self.qos,
                self.retain,
                payload_data.into_inner().freeze(),
            )
            .await
            .map_err(anyhow::Error::from)
    }
}

impl<T> State<T>
where
    T: fmt::Debug + PartialEq + Serialize + Clone,
{
    /// Combine [update] and [publish] into a single function.
    pub(crate) async fn publish_if_update(
        &mut self,
        value: T,
        client: &mut AsyncClient,
    ) -> anyhow::Result<bool> {
        if self.update(value).await {
            self.publish(client).await.and(Ok(true))
        } else {
            debug!(value = ?self.value, "Skipping update of unchanged value");
            Ok(false)
        }
    }
}

impl<T> fmt::Debug for State<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("value", &self.value)
            .field("topic", &self.topic())
            .field("retain", &self.retain)
            .field("qos", &self.qos)
            .finish()
    }
}
