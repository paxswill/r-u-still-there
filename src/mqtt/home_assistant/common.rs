// SPDX-License-Identifier: GPL-3.0-or-later
use std::borrow::Borrow;
use std::collections::HashSet;
use std::iter::Iterator;

use serde::{Deserialize, Serialize};

use super::{device::Device, util::is_default};
use crate::{default_newtype, default_string};

default_string!(PayloadAvailable, "online");
default_string!(PayloadNotAvailable, "offline");

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct AvailabilityTopic {
    #[serde(alias = "pl_avail", default, skip_serializing_if = "is_default")]
    pub payload_available: PayloadAvailable,

    #[serde(alias = "pl_not_avail", default, skip_serializing_if = "is_default")]
    pub payload_not_available: PayloadNotAvailable,

    #[serde(alias = "t")]
    pub topic: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AvailabilityMode {
    All,
    Any,
    Latest,
}

impl Default for AvailabilityMode {
    fn default() -> Self {
        Self::Latest
    }
}

// TODO: encode this as a enum of numbers (and duplicate mqttbytes::QoS in the process)
default_newtype!(SensorQoS, u8, 0);
default_newtype!(ForceUpdate, bool, false);
default_newtype!(EnabledByDefault, bool, true);

/// Settings common to any MQTT device.
///
/// This type is generic over the type of owning reference (I think that's the right term) to a
/// [Device] it has. The default is to own a [Device] outright, but the intention os to allow [Rc],
/// [Arc], or any other type that implements [Borrow] to be used as required.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EntityConfig<P = Device>
where
    P: Borrow<Device> + Default + PartialEq,
{
    #[serde(alias = "avty", default, skip_serializing_if = "is_default")]
    availability: HashSet<AvailabilityTopic>,

    #[serde(alias = "avty_mode", default, skip_serializing_if = "is_default")]
    pub availability_mode: AvailabilityMode,

    // NOTE: This is an Rc, that is serialized. This requires opting in to Rc/Arc serialization
    // with serde, as ref counted types aren't completely preserved. In this case that's ok, as the
    // ref counting is to try to keep memory usage/allocations down when doing a one-time
    // configuration on start up.
    #[serde(alias = "dev", default, skip_serializing_if = "is_default")]
    device: P,

    #[serde(default, skip_serializing_if = "is_default")]
    pub enabled_by_default: EnabledByDefault,

    #[serde(alias = "exp_aft", default, skip_serializing_if = "is_default")]
    pub expire_after: Option<u32>,

    #[serde(alias = "frc_upd", default, skip_serializing_if = "is_default")]
    pub force_update: ForceUpdate,

    #[serde(alias = "ic", default, skip_serializing_if = "is_default")]
    pub icon: Option<String>,

    #[serde(alias = "json_attr_tpl", default, skip_serializing_if = "is_default")]
    pub json_attributes_template: Option<String>,

    #[serde(alias = "json_attr_t", default, skip_serializing_if = "is_default")]
    pub json_attributes_topic: Option<String>,

    // Not including 'name', as the default value for that is specific to the type of device
    #[serde(alias = "pl_avail", default, skip_serializing_if = "is_default")]
    pub payload_available: PayloadAvailable,

    #[serde(alias = "pl_not_avail", default, skip_serializing_if = "is_default")]
    pub payload_not_available: PayloadNotAvailable,

    #[serde(default, skip_serializing_if = "is_default")]
    pub qos: SensorQoS,

    #[serde(alias = "stat_t")]
    pub state_topic: String,

    #[serde(alias = "uniq_id", default, skip_serializing_if = "is_default")]
    pub unique_id: Option<String>,

    #[serde(alias = "val_tpl", default, skip_serializing_if = "is_default")]
    pub value_template: Option<String>,
}

impl<P> EntityConfig<P>
where
    P: Borrow<Device> + Default + PartialEq,
{
    pub fn new_with_state_and_device<S>(state_topic: S, device: P) -> Self
    where
        S: Into<String>,
    {
        Self {
            availability: HashSet::default(),
            availability_mode: AvailabilityMode::default(),
            device,
            enabled_by_default: EnabledByDefault::default(),
            expire_after: None,
            force_update: ForceUpdate::default(),
            icon: None,
            json_attributes_template: None,
            json_attributes_topic: None,
            payload_available: PayloadAvailable::default(),
            payload_not_available: PayloadNotAvailable::default(),
            qos: SensorQoS::default(),
            state_topic: state_topic.into(),
            unique_id: None,
            value_template: None,
        }
    }

    pub fn add_availability_topic_with_values<A, N>(
        &mut self,
        topic: String,
        available: A,
        not_available: N,
    ) where
        A: Into<PayloadAvailable>,
        N: Into<PayloadNotAvailable>,
    {
        self.availability.insert(AvailabilityTopic {
            payload_available: available.into(),
            payload_not_available: not_available.into(),
            topic,
        });
    }

    pub fn add_availability_topic(&mut self, topic: String) {
        self.add_availability_topic_with_values(
            topic,
            PayloadAvailable::default(),
            PayloadNotAvailable::default(),
        );
    }

    pub fn set_availability_topic_with_values<A, N>(
        &mut self,
        topic: String,
        available: A,
        not_available: N,
    ) where
        A: Into<PayloadAvailable>,
        N: Into<PayloadNotAvailable>,
    {
        self.availability.clear();
        self.add_availability_topic_with_values(topic, available, not_available);
    }

    pub fn set_availability_topic(&mut self, topic: String) {
        self.set_availability_topic_with_values(
            topic,
            PayloadAvailable::default(),
            PayloadNotAvailable::default(),
        );
    }

    pub fn availability_topics(&self) -> impl Iterator<Item = &AvailabilityTopic> {
        self.availability.iter()
    }

    pub fn device(&self) -> &Device {
        self.device.borrow()
    }

    pub fn set_device(&mut self, device: P) {
        self.device = device;
    }
}
