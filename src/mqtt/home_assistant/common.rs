// SPDX-License-Identifier: GPL-3.0-or-later
use std::cell::{Ref, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use paste::paste;
use serde::{Deserialize, Serialize};

use crate::{default_newtype, default_string};
use super::{device::Device, is_default};

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

impl AvailabilityTopic {
    pub fn new(topic: String) -> Self {
        Self {
            payload_available: PayloadAvailable::default(),
            payload_not_available: PayloadNotAvailable::default(),
            topic: topic,
        }
    }
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

/// Settings common to any MQTT device
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MqttConfig {
    #[serde(alias = "avty", default, skip_serializing_if = "is_default")]
    pub availability: HashSet<AvailabilityTopic>,

    #[serde(alias = "avty_mode", default, skip_serializing_if = "is_default")]
    pub(super) availability_mode: AvailabilityMode,

    // NOTE: This is an Rc, that is serialized. This requires opting in to Rc/Arc serialization
    // with serde, as ref counted types aren't completely preserved. In this case that's ok, as the
    // ref counting is to try to keep memory usage/allocations down when doing a one-time
    // configuration on start up.
    #[serde(alias = "dev", default, skip_serializing_if = "is_default")]
    pub(super) device: Rc<RefCell<Device>>,

    #[serde(alias = "exp_aft", default, skip_serializing_if = "is_default")]
    pub(super) expire_after: Option<u32>,

    #[serde(alias = "exp_aft", default, skip_serializing_if = "is_default")]
    pub(super) force_update: ForceUpdate,

    #[serde(alias = "ic", default, skip_serializing_if = "is_default")]
    pub(super) icon: Option<String>,

    #[serde(alias = "json_attr_tpl", default, skip_serializing_if = "is_default")]
    pub(super) json_attributes_template: Option<String>,

    #[serde(alias = "json_attr_t", default, skip_serializing_if = "is_default")]
    pub(super) json_attributes_topic: Option<String>,

    // Not including 'name', as the default value for that is specific to the type of device
    #[serde(alias = "pl_avail", default, skip_serializing_if = "is_default")]
    pub(super) payload_available: PayloadAvailable,

    #[serde(alias = "pl_not_avail", default, skip_serializing_if = "is_default")]
    pub(super) payload_not_available: PayloadNotAvailable,

    #[serde(default, skip_serializing_if = "is_default")]
    pub(super) qos: SensorQoS,

    #[serde(alias = "stat_t")]
    pub(super) state_topic: String,

    #[serde(alias = "uniq_id", default, skip_serializing_if = "is_default")]
    pub(super) unique_id: Option<String>,

    #[serde(alias = "val_tpl", default, skip_serializing_if = "is_default")]
    pub(super) value_template: Option<String>,
}

impl MqttConfig {
    pub fn new_with_state_topic<P: Into<String>>(state_topic: P) -> Self {
        Self {
            availability: HashSet::default(),
            availability_mode: AvailabilityMode::default(),
            device: Rc::new(RefCell::new(Device::default())),
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

    pub fn clone_with_state<P: Into<String>>(&self, new_state: P) -> Self {
        Self {
            state_topic: new_state.into(),
            ..self.clone()
        }
    }
}
