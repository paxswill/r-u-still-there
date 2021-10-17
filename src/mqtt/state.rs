// SPDX-License-Identifier: GPL-3.0-or-later
use std::borrow::Borrow;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;

use anyhow::anyhow;
use futures::sink::{unfold, Sink};
use rumqttc::{AsyncClient, QoS};
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::debug;

use super::home_assistant as hass;
use super::serialize::serialize;

// Making DiscoveryValue implement those traits so I don't have to keep adding them to where clauses.
pub(crate) trait DiscoveryValue<D = hass::Device>:
    Default + Serialize + Send + Sync
where
    D: Borrow<hass::Device>,
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

#[derive(Debug)]
pub(crate) struct InnerState<T, D>
where
    T: DiscoveryValue<D>,
    D: Borrow<hass::Device> + Default,
{
    client: Arc<Mutex<AsyncClient>>,
    topic: String,
    retain: bool,
    qos: QoS,
    value_phantom: PhantomData<T>,
    device_phantom: PhantomData<D>,
}

impl<T, D> InnerState<T, D>
where
    T: DiscoveryValue<D>,
    D: Borrow<hass::Device> + Default,
{
    fn new(client: Arc<Mutex<AsyncClient>>, topic: String, retain: bool, qos: QoS) -> Self {
        Self {
            client,
            topic,
            retain,
            qos,
            value_phantom: PhantomData,
            device_phantom: PhantomData,
        }
    }

    /// Publish the current state.
    async fn publish(&self, value: T) -> anyhow::Result<()>
    where
        T: fmt::Debug,
    {
        let payload_data = serialize(&value)?;
        debug!(?value, ?self.topic, "Publishing value to topic");
        self.client
            .lock()
            .await
            .publish(&self.topic, self.qos, self.retain, payload_data)
            .await
            .map_err(anyhow::Error::from)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum State<T, D>
where
    T: DiscoveryValue<D>,
    D: Borrow<hass::Device> + Default,
{
    Basic {
        inner: Arc<InnerState<T, D>>,
    },
    Discoverable {
        inner: Arc<InnerState<T, D>>,
        name: String,
        prefix: String,
        device: D,
    },
}

impl<T, D> State<T, D>
where
    T: DiscoveryValue<D>,
    D: Borrow<hass::Device> + Default,
{
    fn inner(&self) -> &Arc<InnerState<T, D>> {
        match self {
            State::Basic { inner, .. } => &inner,
            State::Discoverable { inner, .. } => &inner,
        }
    }

    /// The full topic path for this state.
    pub(crate) fn topic(&self) -> &str {
        &self.inner().topic
    }

    /// Publish the current state.
    pub(crate) async fn publish(&self, value: T) -> anyhow::Result<()>
    where
        T: fmt::Debug,
    {
        self.inner().publish(value).await
    }
}

impl<T, D> State<T, D>
where
    T: DiscoveryValue<D> + fmt::Debug,
    <T as DiscoveryValue<D>>::Config: fmt::Debug,
    D: Borrow<hass::Device> + Clone + Default,
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
    ) -> Self {
        Self::Basic {
            inner: Arc::new(InnerState::new(
                client,
                Self::topic_for(prefix, device_name, entity_name),
                retain,
                qos,
            )),
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
            inner: Arc::new(InnerState::new(
                client,
                Self::topic_for(&prefix, &device_name, &name),
                retain,
                qos,
            )),
            name,
            prefix,
            device,
        }
    }

    // Trying to implement Sink on State itself is such a pain in the ass, I'm punting and having a
    // separate function do all the magic.
    pub(crate) fn sink(&self) -> impl Sink<T, Error = anyhow::Error> {
        unfold(Arc::clone(&self.inner()), |inner, value| async move {
            inner.publish(value).await?;
            Ok(inner)
        })
    }

    pub(crate) fn discovery_config<A>(&self, availability_topic: A) -> anyhow::Result<T::Config>
    where
        A: Into<String>,
    {
        match self {
            State::Basic { .. } => Err(anyhow!("Basic states don't have discovery configurations")),
            State::Discoverable { name, device, .. } => {
                let entity_name = device.borrow().name.as_deref().map_or_else(
                    || name.clone(),
                    |device_name| [device_name.borrow(), name.borrow()].join(" "),
                );
                let unique_id = self.unique_id().unwrap();
                let config = T::home_assistant_config(
                    D::clone(&device),
                    self.topic().into(),
                    availability_topic.into(),
                    entity_name,
                    unique_id,
                );
                Ok(config)
            }
        }
    }

    pub(crate) fn discovery_topic<S>(&self, home_assistant_prefix: S) -> anyhow::Result<String>
    where
        S: Into<String>,
    {
        match self {
            State::Basic { .. } => Err(anyhow!("Basic states don't have discovery topics")),
            State::Discoverable { .. } => {
                let unique_id = self.unique_id().unwrap();
                let home_assistant_prefix = home_assistant_prefix.into();
                let config_topic = [
                    &home_assistant_prefix,
                    &T::component_type().to_string(),
                    &unique_id,
                    "config",
                ]
                .join("/");
                Ok(config_topic)
            }
        }
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
            State::Discoverable { inner, .. } => {
                let config_topic = self.discovery_topic(home_assistant_prefix)?;
                let config = self.discovery_config(availability_topic)?;
                debug!(?config, "Publishing Home Assistant discovery config");
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
