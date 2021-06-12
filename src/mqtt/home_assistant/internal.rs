// SPDX-License-Identifier: GPL-3.0-or-later
use std::cell::{Ref, RefCell};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::mem::discriminant;
use std::rc::Rc;

use mac_address::MacAddress;
use paste::paste;
use serde::de::{Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::*;

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

#[derive(Clone, Copy, PartialEq)]
pub enum Component {
    /// Binary sensors
    BinarySensor,

    /// Non-binary sensors, with many values
    Sensor,

    /// Cameras. Not available yet.
    Camera,
}

impl std::string::ToString for Component {
    /// Returns the name of this component for use by Home Assistant.
    fn to_string(&self) -> String {
        match self {
            Component::BinarySensor => "binary_sensor",
            Component::Sensor => "sensor",
            Component::Camera => "camera",
        }
        .to_string()
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

#[macro_export]
macro_rules! expose_inner {
    ($name:ident, $typ:ty) => {
        pub fn $name(&self) -> &$typ {
            &self.$name
        }
        paste! {
            pub fn [<set_ $name>](&mut self, new_value: $typ) {
                self.$name = new_value
            }
        }
    };
    ($inner_name:ident, $name:ident, $typ:ty) => {
        pub fn $name(&self) -> &$typ {
            &self.$inner_name.$name
        }
        paste! {
            pub fn [<set_ $name>](&mut self, new_value: $typ) {
                self.$inner_name.$name = new_value
            }
        }
    };
}

#[macro_export]
macro_rules! expose_mqtt_config {
    ($name:ident, $typ:ty) => {
        expose_inner!(mqtt, $name, $typ);
    };
}

#[macro_export]
macro_rules! expose_device_config {
    ($name:ident, $typ:ty) => {
        paste! {
            pub fn [<device_ $name>](&self) -> Ref<'_, $typ> {
                Ref::map(self.mqtt.device.borrow(), |d| &d.$name)
            }
            pub fn [<set_device_ $name>](&mut self, new_value: $typ) {
                self.mqtt.device.borrow_mut().$name = new_value
            }
        }
    };
}

#[macro_export]
macro_rules! expose_common {
    () => {
        pub fn availability_topics(&self) -> &HashSet<AvailabilityTopic> {
            &self.mqtt.availability
        }
        pub fn add_availability_topic(&mut self, topic: String) {
            self.mqtt.availability.insert(AvailabilityTopic::new(topic));
        }
        expose_mqtt_config!(availability_mode, AvailabilityMode);
        // Device?
        expose_mqtt_config!(expire_after, Option<u32>);
        expose_mqtt_config!(force_update, ForceUpdate);
        expose_mqtt_config!(icon, Option<String>);
        expose_mqtt_config!(json_attributes_template, Option<String>);
        expose_mqtt_config!(json_attributes_topic, Option<String>);
        expose_mqtt_config!(payload_available, PayloadAvailable);
        expose_mqtt_config!(payload_not_available, PayloadNotAvailable);
        expose_mqtt_config!(qos, SensorQoS);
        expose_mqtt_config!(state_topic, String);
        expose_mqtt_config!(unique_id, Option<String>);
        expose_mqtt_config!(value_template, Option<String>);

        /*
        //expose_device_config!(connections, Option<Connection>);
        pub fn device_mac(&self) -> Option<Ref<'_, &String>> {
            let connections: Ref<'_, Option<Connection>> = Ref::map(self.mqtt.device.borrow(), |d| &d.connections);
            // It's surprisingly tricky to convert a Ref<'_, Option<T>> to Option<Ref<'_, T>>
            if connections.is_none() {
                Some(Ref::map(connections, |c| {

                }))
            } else {
                None
            }
        }
        */
        expose_device_config!(identifiers, HashSet<String>);
        expose_device_config!(manufacturer, Option<String>);
        expose_device_config!(model, Option<String>);
        expose_device_config!(name, Option<String>);
        expose_device_config!(suggested_area, Option<String>);
        expose_device_config!(sw_version, Option<String>);
        expose_device_config!(via_device, Option<String>);
    };
}

// Only defining a few of the classes for now. If I break this out into a library, this should be
// expanded to cover all of the device classes.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinarySensorClass {
    None,
    Battery,
    Connectivity,
    Occupancy,
}

impl Default for BinarySensorClass {
    fn default() -> Self {
        Self::None
    }
}

default_string!(BinarySensorName, "MQTT Binary Sensor");
default_string!(PayloadOff, "OFF");
default_string!(PayloadOn, "ON");

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BinarySensor {
    #[serde(flatten)]
    mqtt: MqttConfig,

    #[serde(alias = "dev_cla", default, skip_serializing_if = "is_default")]
    device_class: BinarySensorClass,

    #[serde(default, skip_serializing_if = "is_default")]
    name: BinarySensorName,

    #[serde(alias = "off_dly", default, skip_serializing_if = "is_default")]
    off_delay: Option<u32>,

    #[serde(alias = "pl_off", default, skip_serializing_if = "is_default")]
    payload_off: PayloadOff,

    #[serde(alias = "pl_on", default, skip_serializing_if = "is_default")]
    payload_on: PayloadOn,
}

#[allow(dead_code)]
impl BinarySensor {
    expose_common!();
    expose_inner!(device_class, BinarySensorClass);
    expose_inner!(off_delay, Option<u32>);
    expose_inner!(payload_off, PayloadOff);
    expose_inner!(payload_on, PayloadOn);

    pub fn new_with_state_topic<P: Into<String>>(state_topic: P) -> Self {
        Self {
            mqtt: MqttConfig::new_with_state_topic(state_topic),
            device_class: BinarySensorClass::default(),
            name: BinarySensorName::default(),
            off_delay: None,
            payload_off: PayloadOff::default(),
            payload_on: PayloadOn::default(),
        }
    }

    pub fn component() -> Component {
        Component::BinarySensor
    }

    pub fn name(&self) -> &String {
        &self.name.0
    }

    pub fn set_name(&mut self, new_name: String) {
        self.name.0 = new_name;
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalogSensorClass {
    None,
    SignalStrength,
    Temperature,
    Timestamp,
}

impl Default for AnalogSensorClass {
    fn default() -> Self {
        Self::None
    }
}

default_string!(AnalogSensorName, "MQTT Sensor");

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AnalogSensor {
    #[serde(flatten)]
    mqtt: MqttConfig,

    #[serde(alias = "dev_cla", default, skip_serializing_if = "is_default")]
    device_class: AnalogSensorClass,

    #[serde(default, skip_serializing_if = "is_default")]
    name: AnalogSensorName,

    #[serde(alias = "unit_of_meas", default, skip_serializing_if = "is_default")]
    unit_of_measurement: Option<String>,
}

#[allow(dead_code)]
impl AnalogSensor {
    expose_common!();
    expose_inner!(device_class, AnalogSensorClass);
    expose_inner!(unit_of_measurement, Option<String>);

    pub fn new_with_state_topic<P: Into<String>>(state_topic: P) -> Self {
        Self {
            mqtt: MqttConfig::new_with_state_topic(state_topic),
            device_class: AnalogSensorClass::default(),
            name: AnalogSensorName::default(),
            unit_of_measurement: None,
        }
    }

    pub fn component() -> Component {
        Component::Sensor
    }

    pub fn name(&self) -> &String {
        &self.name.0
    }

    pub fn set_name(&mut self, new_name: String) {
        self.name.0 = new_name;
    }
}
