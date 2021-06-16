// SPDX-License-Identifier: GPL-3.0-or-later
use std::fmt;

use bytes::{BufMut, BytesMut};
use rumqttc::{AsyncClient, QoS};
use serde::Serialize;
use tracing::debug;

/// A container for managing MQTT topic state.
pub(crate) struct State<T> {
    value: T,
    topic: String,
    retain: bool,
    qos: QoS,
}

impl<T> State<T> {
    pub(crate) fn new<S>(value: T, topic: S, retain: bool, qos: QoS) -> Self
    where
        S: Into<String>
    {
        Self {
            value,
            topic: topic.into(),
            retain,
            qos
        }
    }

    /// Get a reference to the current state of this topic
    pub(crate) fn current(&self) -> &T {
        &self.value
    }

    /// The full topic path for this state.
    pub(crate) fn topic(&self) -> &str {
        &self.topic
    }
}

impl<T> State<T>
where
    T: Default
{
    pub(crate) fn new_default_at<S>(topic: S) -> Self
    where
        S: Into<String>
    {
        Self::new(T::default(), topic, true, QoS::AtLeastOnce)
    }

}

impl<T> State<T>
where
    T: PartialEq
{
    /// Update the state of this topic, returning whether or not the new value is different than
    /// the old one.
    pub(crate) fn update(&mut self, value: T) -> bool {
        if self.value != value {
            self.value = value;
            true
        } else {
            false
        }
    }
}

impl<T> State<T>
where
    T: Serialize
{
    /// Publish the current state using the provided client.
    pub(crate) async fn publish(&self, client: &mut AsyncClient) -> anyhow::Result<()> {
        let mut payload_data = BytesMut::new().writer();
        serde_json::to_writer(&mut payload_data, self.current())?;
        client
            .publish_bytes(
                self.topic(),
                self.qos,
                self.retain,
        payload_data.into_inner().freeze()
            )
            .await.map_err(anyhow::Error::from)
    }
}

impl<T> State<T>
where
    T: fmt::Debug + PartialEq + Serialize
{
    /// Combine [update] and [publish] into a single function.
    pub(crate) async fn publish_if_update(&mut self, value: T, client: &mut AsyncClient) -> anyhow::Result<bool> {
        if self.update(value) {
            self.publish(client).await.and(Ok(true))
        } else {
            debug!(value = ?self.value, "Skipping update of unchanged value");
            Ok(false)
        }
    }
}

impl<T> fmt::Debug for State<T>
where
    T: fmt::Debug
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("value", self.current())
            .field("topic", &self.topic())
            .field("retain", &self.retain)
            .field("qos", &self.qos)
            .finish()
    }
}
