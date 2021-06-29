// SPDX-License-Identifier: GPL-3.0-or-later
use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use futures::sink::{unfold, Sink};
use rumqttc::{AsyncClient, QoS};
use serde::{Serialize, Serializer};
use tokio::sync::{Mutex, RwLock, RwLockReadGuard};
use tracing::debug;

use super::home_assistant as hass;
use super::serialize::serialize;

#[derive(Debug)]
pub(crate) struct InnerState<T>
where
    T: fmt::Debug + Send + Sync,
{
    client: Arc<Mutex<AsyncClient>>,
    value: RwLock<T>,
    topic: String,
    retain: bool,
    qos: QoS,
}

impl<T> InnerState<T>
where
    T: fmt::Debug + Send + Sync,
    T: Serialize + PartialEq,
{
    /// Publish the current state.
    async fn publish(&self) -> anyhow::Result<()> {
        let value = SerializeRwLockGuard::from(self.value.read().await);
        let payload_data = serialize(&value)?;
        debug!(?value, ?self.topic, "Publishing value to topic");
        self.client
            .lock()
            .await
            .publish(&self.topic, self.qos, self.retain, payload_data)
            .await
            .map_err(anyhow::Error::from)
    }

    /// Update the state of this topic, returning whether or not the new value is different than
    /// the old one.
    async fn update(&self, new_value: T) -> bool {
        let mut current_value = self.value.write().await;
        if *current_value != new_value {
            *current_value = new_value;
            true
        } else {
            false
        }
    }

    /// Combine [update] and [publish] into a single function.
    async fn publish_if_update(&self, value: T) -> anyhow::Result<bool> {
        if self.update(value).await {
            self.publish().await.and(Ok(true))
        } else {
            let value = self.value.read().await;
            debug!(?value, ?self.topic, "Skipped publishing update to topic");
            Ok(false)
        }
    }
}

#[derive(Clone)]
pub(crate) enum State<T, D>
where
    T: DiscoveryValue<D> + fmt::Debug + PartialEq,
    D: Borrow<hass::Device> + Default + PartialEq,
    D: Send + Sync,
{
    Basic {
        inner: Arc<InnerState<T>>,
    },
    Discoverable {
        inner: Arc<InnerState<T>>,
        name: String,
        prefix: String,
        device: D,
    },
}

impl<T, D> State<T, D>
where
    T: DiscoveryValue<D> + fmt::Debug + PartialEq,
    D: Borrow<hass::Device> + Clone + Default + PartialEq,
    D: Send + Sync,
{
    fn topic_for(prefix: &str, device_name: &str, entity_name: &str) -> String {
        [prefix.deref(), device_name.deref(), entity_name.deref()].join("/")
    }

    pub(crate) fn new(
        client: Arc<Mutex<AsyncClient>>,
        prefix: &str,
        device_name: &str,
        entity_name: &str,
        retain: bool,
        qos: QoS,
    ) -> Self
where {
        Self::Basic {
            inner: Arc::new(InnerState {
                client,
                value: RwLock::new(T::default()),
                topic: Self::topic_for(prefix, device_name, entity_name),
                retain,
                qos,
            }),
        }
    }

    pub(crate) fn new_discoverable(
        client: Arc<Mutex<AsyncClient>>,
        device: D,
        prefix: &str,
        entity_name: &str,
        retain: bool,
        qos: QoS,
    ) -> Self {
        let name = entity_name.to_string();
        let prefix = prefix.to_string();
        let device_name = device.borrow().name.clone().unwrap_or_else(|| {
            device
                .borrow()
                .identifiers()
                .next()
                .expect("The device to have at least one ID")
                .to_string()
        });
        Self::Discoverable {
            inner: Arc::new(InnerState {
                client,
                value: RwLock::new(T::default()),
                topic: Self::topic_for(&prefix, &device_name, &name),
                retain,
                qos,
            }),
            name,
            prefix,
            device,
        }
    }

    fn inner(&self) -> &Arc<InnerState<T>> {
        match self {
            State::Basic { inner, .. } => &inner,
            State::Discoverable { inner, .. } => &inner,
        }
    }

    // Trying to implement Sink on State itself is such a pain in the ass, I'm punting and having a
    // separate function do all the magic.
    pub(crate) fn sink(&self) -> impl Sink<T, Error = anyhow::Error> {
        unfold(Arc::clone(&self.inner()), |inner, value| async move {
            inner.publish_if_update(value).await?;
            Ok(inner)
        })
    }

    /// The full topic path for this state.
    pub(crate) fn topic(&self) -> &str {
        &self.inner().topic
    }

    /// Publish the current state.
    pub(crate) async fn publish(&self) -> anyhow::Result<()> {
        self.inner().publish().await
    }

    /// Update the state of this topic, returning whether or not the new value is different than
    /// the old one.
    pub(crate) async fn update(&self, new_value: T) -> bool {
        self.inner().update(new_value).await
    }

    /// Combine [update] and [publish] into a single function.
    pub(crate) async fn publish_if_update(&self, value: T) -> anyhow::Result<bool> {
        self.inner().publish_if_update(value).await
    }

    pub(crate) async fn publish_home_assistant_discovery<S, A>(
        &self,
        home_assistant_prefix: S,
        availability_topic: A,
    ) -> anyhow::Result<()>
    where
        S: Into<String>,
        A: Into<String>,
    {
        match self {
            State::Basic { .. } => Ok(()),
            State::Discoverable {
                inner,
                name,
                device,
                ..
            } => {
                let entity_name = device.borrow().name.as_deref().map_or_else(
                    || name.clone(),
                    |device_name| [device_name.borrow(), name.borrow()].join(" "),
                );
                let unique_id = self.unique_id().unwrap();
                let home_assistant_prefix = home_assistant_prefix.into();
                let config_topic = [
                    &home_assistant_prefix,
                    &T::component_type().to_string(),
                    &unique_id,
                    "config",
                ]
                .join("/");
                let config = T::home_assistant_config(
                    D::clone(&device),
                    self.topic().into(),
                    availability_topic.into(),
                    entity_name,
                    unique_id,
                );
                let payload = serialize(&config)?;
                inner
                    .client
                    .lock()
                    .await
                    // Discovery messages should be retained
                    .publish(config_topic, QoS::AtLeastOnce, true, payload)
                    .await
                    .map_err(anyhow::Error::from)
            }
        }
    }

    fn unique_id(&self) -> Option<String> {
        match self {
            State::Basic { .. } => None,
            State::Discoverable {
                device,
                name,
                prefix,
                ..
            } => device
                .borrow()
                .identifiers()
                .next()
                .map(|id| [id, &Self::machine_name(&name), &prefix].join("_")),
        }
    }

    fn machine_name(name: &str) -> String {
        let lowercased = name.to_lowercase();
        let components: Vec<_> = lowercased.split_whitespace().collect();
        components.join("_").replace('/', "_")
    }
}

impl<T, D> fmt::Debug for State<T, D>
where
    T: DiscoveryValue<D> + fmt::Debug + PartialEq,
    D: Borrow<hass::Device> + Clone + Default + PartialEq,
    D: Send + Sync,
    D: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            State::Basic { inner } => f
                .debug_struct("State::Basic")
                .field("inner", &inner)
                .finish(),
            State::Discoverable {
                inner,
                name,
                prefix,
                device,
            } => f
                .debug_struct("State::Discoverable")
                .field("inner", &inner)
                .field("name", &name)
                .field("prefix", &prefix)
                .field("device", &device)
                .finish(),
        }
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

// Making DiscoveryValue implement those traits so I don't have to keep adding them to where clauses.
pub(crate) trait DiscoveryValue<D = hass::Device>:
    Default + Serialize + Send + Sync
where
    D: Borrow<hass::Device> + Default + PartialEq,
{
    type Config: Serialize;

    /// If the value should be retained when published.
    fn retained() -> bool;

    fn component_type() -> hass::Component;

    fn home_assistant_config(
        device: D,
        state_topic: String,
        availability_topic: String,
        name: String,
        unique_id: String,
    ) -> Self::Config;
}
