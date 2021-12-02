// SPDX-License-Identifier: GPL-3.0-or-later
use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;

use futures::sink::{unfold, Sink};
use rumqttc::QoS;
use serde::Serialize;
use tracing::debug;

use super::client::MqttSender;
use super::home_assistant as hass;

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

#[derive(Clone, Debug)]
pub(crate) struct InnerState {
    sender: MqttSender,
    topic: String,
    retain: bool,
    qos: QoS,
}

impl InnerState {
    fn new(sender: MqttSender, topic: String, retain: bool, qos: QoS) -> Self {
        Self {
            sender,
            topic,
            retain,
            qos,
        }
    }

    /// Publish the current state.
    async fn publish<T, D>(&mut self, value: T) -> anyhow::Result<()>
    where
        T: fmt::Debug + DiscoveryValue<D>,
        D: Borrow<hass::Device> + Default,
    {
        debug!(?value, ?self.topic, "Publishing value to topic");
        self.sender
            .publish_when_connected(self.topic.clone(), self.qos, &value, self.retain)
            .await
    }
}

#[derive(Clone, Debug)]
pub(crate) enum State<D>
where
    D: Borrow<hass::Device> + Default,
{
    Basic {
        inner: InnerState,
    },
    Discoverable {
        inner: InnerState,
        name: String,
        prefix: String,
        device: D,
    },
}

impl<D> State<D>
where
    D: Borrow<hass::Device> + Default,
{
    fn inner(&self) -> &InnerState {
        match self {
            State::Basic { inner, .. } => inner,
            State::Discoverable { inner, .. } => inner,
        }
    }

    fn inner_mut(&mut self) -> &mut InnerState {
        match self {
            State::Basic { inner, .. } => inner,
            State::Discoverable { inner, .. } => inner,
        }
    }

    /// The full topic path for this state.
    pub(crate) fn topic(&self) -> &str {
        &self.inner().topic
    }

    /// Returns `true` if the state is [`Basic`].
    ///
    /// [`Basic`]: State::Basic
    pub(crate) fn is_basic(&self) -> bool {
        matches!(self, Self::Basic { .. })
    }

    /// Returns `true` if the state is [`Discoverable`].
    ///
    /// [`Discoverable`]: State::Discoverable
    pub(crate) fn is_discoverable(&self) -> bool {
        matches!(self, Self::Discoverable { .. })
    }
}

impl<D> State<D>
where
    D: Borrow<hass::Device> + Clone + Default,
    D: Send + Sync,
{
    fn topic_for(prefix: &str, device_name: &str, entity_name: &str) -> String {
        [prefix.deref(), device_name.deref(), entity_name.deref()].join("/")
    }

    pub(crate) fn new(
        sender: MqttSender,
        prefix: &str,
        device_name: &str,
        entity_name: &str,
        retain: bool,
        qos: QoS,
    ) -> Self {
        Self::Basic {
            inner: InnerState::new(
                sender,
                Self::topic_for(prefix, device_name, entity_name),
                retain,
                qos,
            ),
        }
    }

    pub(crate) fn new_discoverable(
        sender: MqttSender,
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
            inner: InnerState::new(
                sender,
                Self::topic_for(&prefix, &device_name, &name),
                retain,
                qos,
            ),
            name,
            prefix,
            device,
        }
    }

    // Trying to implement Sink on State itself is such a pain in the ass, I'm punting and having a
    // separate function do all the magic.
    pub(crate) fn sink<T>(&self) -> impl Sink<T, Error = anyhow::Error>
    where
        T: DiscoveryValue<D> + fmt::Debug,
        <T as DiscoveryValue<D>>::Config: fmt::Debug,
    {
        let unfold_state = self.inner().clone();
        unfold(unfold_state, move |mut inner, value| async move {
            inner.publish(value).await?;
            Ok(inner)
        })
    }

    pub(crate) fn discovery_config<T>(&self, availability_topic: &str) -> Option<T::Config>
    where
        T: DiscoveryValue<D> + fmt::Debug,
        <T as DiscoveryValue<D>>::Config: fmt::Debug,
    {
        match self {
            State::Basic { .. } => None,
            State::Discoverable { name, device, .. } => {
                let entity_name = device.borrow().name.as_deref().map_or_else(
                    || name.clone(),
                    |device_name| [device_name.borrow(), name.borrow()].join(" "),
                );
                let unique_id = self.unique_id().unwrap();
                let config = T::home_assistant_config(
                    D::clone(device),
                    self.topic().into(),
                    availability_topic.to_string(),
                    entity_name,
                    unique_id,
                );
                Some(config)
            }
        }
    }

    pub(crate) fn discovery_topic<T>(&self, home_assistant_prefix: &str) -> Option<String>
    where
        T: DiscoveryValue<D> + fmt::Debug,
        <T as DiscoveryValue<D>>::Config: fmt::Debug,
    {
        match self {
            State::Basic { .. } => None,
            State::Discoverable { .. } => {
                let unique_id = self.unique_id().unwrap();
                let home_assistant_prefix = home_assistant_prefix.to_string();
                let config_topic = [
                    &home_assistant_prefix,
                    &T::component_type().to_string(),
                    &unique_id,
                    "config",
                ]
                .join("/");
                Some(config_topic)
            }
        }
    }

    pub(crate) async fn publish_home_assistant_discovery<T>(
        &mut self,
        home_assistant_prefix: &str,
        availability_topic: &str,
    ) -> anyhow::Result<()>
    where
        T: DiscoveryValue<D> + fmt::Debug,
        <T as DiscoveryValue<D>>::Config: fmt::Debug,
    {
        let discovery_topic = self.discovery_topic::<T>(home_assistant_prefix);
        let discovery_config = self.discovery_config::<T>(availability_topic);
        if let Some((config_topic, config)) = discovery_topic.zip(discovery_config) {
            debug!(?config, "Publishing Home Assistant discovery config");
            self.inner_mut()
                .sender
                // Discovery messages should be retained
                .enqueue_publish(config_topic, QoS::AtLeastOnce, &config, true)
                .await
                .map_err(anyhow::Error::from)
        } else {
            Ok(())
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
                .map(|id| [id, &Self::machine_name(name), prefix].join("_")),
        }
    }

    fn machine_name(name: &str) -> String {
        let lowercased = name.to_lowercase();
        let components: Vec<_> = lowercased.split_whitespace().collect();
        components.join("_").replace('/', "_")
    }
}
