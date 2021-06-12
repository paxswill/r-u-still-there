// SPDX-License-Identifier: GPL-3.0-or-later
use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::mem::discriminant;
use std::rc::Rc;

use mac_address::MacAddress;
use serde::de::{Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Skip serializing a field if the current value is the same as the default.
// Code taken from https://mth.st/blog/skip-default/
pub fn is_default<T: Default + PartialEq>(val: &T) -> bool {
    val == &T::default()
}

#[macro_export]
macro_rules! default_newtype {
    ($name:ident, $wrapped_type:ty, $default:literal) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
        pub struct $name(pub $wrapped_type);
        impl Default for $name {
            fn default() -> Self {
                $name($default.into())
            }
        }
        impl From<$name> for $wrapped_type {
            fn from(wrapper: $name) -> Self {
                wrapper.0
            }
        }
        impl From<$wrapped_type> for $name {
            fn from(wrapped: $wrapped_type) -> Self {
                $name(wrapped)
            }
        }
    };
}

#[macro_export]
macro_rules! default_string {
    ($name:ident, $default:literal) => {
        default_newtype!($name, String, $default);
    };
}

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

/// The types of connections that can be associated with a device in the Home Assistant device
/// registry.
///
/// There isn't a great reference for what kinds of connections are supported. The docs give MAC
/// addresses as an example, but by peeking in the source we can see that UPnP and Zigbee IDs are
/// also supported.
#[derive(Clone, Debug, PartialEq)]
pub enum Connection {
    MacAddress(MacAddress),
    // TODO: see if this actually matches the spec. This will also be a bit difficult as I'm not
    // sure of any integrations that actually use this :/
    UPnP(Uuid),
    Zigbee(String),
}
// MacAddress doesn't implement Eq or Hash, so we get to implement (or mark) those ourselves.
impl std::cmp::Eq for Connection {}
impl Hash for Connection {
    fn hash<H: Hasher>(&self, state: &mut H) {
        discriminant(&self).hash(state);
        match self {
            Connection::MacAddress(mac) => {
                mac.bytes().hash(state);
            }
            Connection::UPnP(upnp) => {
                upnp.hash(state);
            }
            Connection::Zigbee(addr) => {
                addr.hash(state);
            }
        }
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ConnectionTag {
    Mac,
    UPnP,
    Zigbee,
}

impl Serialize for Connection {
    /// Serialize a Connection as a 2-tuple
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tuple_serializer = serializer.serialize_tuple(2)?;
        match self {
            Connection::MacAddress(mac) => {
                tuple_serializer.serialize_element(&ConnectionTag::Mac)?;
                tuple_serializer.serialize_element(mac)?;
            }
            Connection::UPnP(uuid) => {
                tuple_serializer.serialize_element(&ConnectionTag::UPnP)?;
                tuple_serializer.serialize_element(uuid)?;
            }
            Connection::Zigbee(addr) => {
                tuple_serializer.serialize_element(&ConnectionTag::Zigbee)?;
                tuple_serializer.serialize_element(addr)?;
            }
        };
        tuple_serializer.end()
    }
}

impl<'de> Deserialize<'de> for Connection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ConnectionVisitor;

        impl<'de> Visitor<'de> for ConnectionVisitor {
            type Value = Connection;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a connection tuple")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                // It's almost missing_field(), but there are multiple fields it could be
                let missing_tag =
                    V::Error::custom("Missing one of the connection type names as a key");
                let tag = map.next_key()?.ok_or_else(|| missing_tag)?;
                let connection_type = match tag {
                    ConnectionTag::Mac => {
                        let mac = map.next_value()?;
                        Connection::MacAddress(mac)
                    }
                    ConnectionTag::UPnP => {
                        let uuid = map.next_value()?;
                        Connection::UPnP(uuid)
                    }
                    ConnectionTag::Zigbee => {
                        let addr = map.next_value()?;
                        Connection::Zigbee(addr)
                    }
                };
                Ok(connection_type)
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                // There should be exactly two elements in the sequence
                let expect_two = || {
                    V::Error::invalid_length(
                        2,
                        &"there should be exactly two elements in a component array",
                    )
                };

                let tag = seq.next_element()?.ok_or_else(expect_two)?;
                let connection_type = match tag {
                    ConnectionTag::Mac => {
                        let mac = seq.next_element()?.ok_or_else(expect_two)?;
                        Connection::MacAddress(mac)
                    }
                    ConnectionTag::UPnP => {
                        let uuid = seq.next_element()?.ok_or_else(expect_two)?;
                        Connection::UPnP(uuid)
                    }
                    ConnectionTag::Zigbee => {
                        let addr = seq.next_element()?.ok_or_else(expect_two)?;
                        Connection::Zigbee(addr)
                    }
                };
                Ok(connection_type)
            }
        }

        deserializer.deserialize_any(ConnectionVisitor)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Device {
    #[serde(alias = "cns", default, skip_serializing_if = "is_default")]
    pub(super) connections: HashSet<Connection>,

    #[serde(alias = "ids", default, skip_serializing_if = "is_default")]
    pub(super) identifiers: HashSet<String>,

    #[serde(alias = "mf", default, skip_serializing_if = "is_default")]
    pub(super) manufacturer: Option<String>,

    #[serde(alias = "mdl", default, skip_serializing_if = "is_default")]
    pub(super) model: Option<String>,

    // No alias for 'name'
    #[serde(default, skip_serializing_if = "is_default")]
    pub(super) name: Option<String>,

    #[serde(alias = "sa", default, skip_serializing_if = "is_default")]
    pub(super) suggested_area: Option<String>,

    #[serde(alias = "sw", default, skip_serializing_if = "is_default")]
    pub(super) sw_version: Option<String>,

    // No alias for 'via_device' either
    #[serde(default, skip_serializing_if = "is_default")]
    pub(super) via_device: Option<String>,
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
