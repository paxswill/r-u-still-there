// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};

/// Skip serializing a field if the current value is the same as the default.
// Code taken from https://mth.st/blog/skip-default/
fn is_default<T: Default + PartialEq>(val: &T) -> bool {
    val == &T::default()
}

macro_rules! default_newtype {
    ($name:ident, $wrapped_type:ty, $default:literal) => {
        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
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

default_string!(PayloadAvailable, "online");
default_string!(PayloadNotAvailable, "offline");

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AvailabilityTopicEntry {
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum Connection {
    #[serde(rename = "mac")]
    MacAddress(String),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Device {
    // TODO: maybe implement the alternative format, a map of types to values
    // Both available formats:
    //     {"connections": {"mac": "de:ad:be:ef:ca:fe"}}
    //     {"connections": ["mac", "de:ad:be:ef:ca:fe"]}
    // TODO: Also verify with HAss docs that this is the actual format
    #[serde(alias = "cns")]
    pub connections: Option<Connection>,

    #[serde(alias = "ids")]
    pub identifiers: Option<Vec<String>>,

    #[serde(alias = "mf")]
    pub manufacturer: Option<String>,

    #[serde(alias = "mdl")]
    pub model: Option<String>,

    // No alias for 'name'
    pub name: Option<String>,

    #[serde(alias = "sa")]
    pub suggested_area: Option<String>,

    #[serde(alias = "sw")]
    pub sw_version: Option<String>,

    // No alias for 'via_device' either
    pub via_device: Option<String>,
}

// TODO: encode this as a enum of numbers (and duplicate mqttbytes::QoS in the process)
default_newtype!(SensorQoS, u8, 0);
default_newtype!(ForceUpdate, bool, false);

/// Settings common to any MQTT device
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MqttConfig {
    #[serde(alias = "avty")]
    pub availability: Option<Vec<AvailabilityTopicEntry>>,

    #[serde(alias = "avty_mode", default, skip_serializing_if = "is_default")]
    pub availability_mode: AvailabilityMode,

    // TODO: define a way so that availability and availability_topic are mutually exclusive
    #[serde(default, skip_serializing_if = "is_default")]
    pub availability_topic: Option<String>,

    #[serde(alias = "dev", default, skip_serializing_if = "is_default")]
    pub device: Device,

    #[serde(alias = "exp_aft", default, skip_serializing_if = "is_default")]
    pub expire_after: Option<u32>,

    #[serde(alias = "exp_aft", default, skip_serializing_if = "is_default")]
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

impl MqttConfig {
    pub fn new_with_state_topic<P: Into<String>>(state_topic: P) -> Self {
        Self {
            availability: None,
            availability_mode: AvailabilityMode::default(),
            availability_topic: None,
            device: Device::default(),
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
}

// Only defining a few of the classes for now. If I break this out into a library, this should be
// expanded to cover all of the device classes.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryDeviceClass {
    None,
    Battery,
    Connectivity,
    Occupancy,
}

impl Default for BinaryDeviceClass {
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
    pub mqtt: MqttConfig,

    #[serde(alias = "dev_cla", default, skip_serializing_if = "is_default")]
    pub device_class: BinaryDeviceClass,

    #[serde(default, skip_serializing_if = "is_default")]
    pub name: BinarySensorName,

    #[serde(alias = "off_dly", default, skip_serializing_if = "is_default")]
    pub off_delay: Option<u32>,

    #[serde(alias = "pl_off", default, skip_serializing_if = "is_default")]
    pub payload_off: PayloadOff,

    #[serde(alias = "pl_on", default, skip_serializing_if = "is_default")]
    pub payload_on: PayloadOn,
}

impl BinarySensor {
    pub fn new_with_state_topic<P: Into<String>>(state_topic: P) -> Self {
        Self {
            mqtt: MqttConfig::new_with_state_topic(state_topic),
            device_class: BinaryDeviceClass::default(),
            name: BinarySensorName::default(),
            off_delay: None,
            payload_off: PayloadOff::default(),
            payload_on: PayloadOn::default(),
        }
    }
}

impl From<&BinarySensor> for Component {
    fn from(sensor: &BinarySensor) -> Self {
        Self::BinarySensor
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
    pub mqtt: MqttConfig,

    #[serde(alias = "dev_cla", default, skip_serializing_if = "is_default")]
    pub device_class: AnalogSensorClass,

    #[serde(default, skip_serializing_if = "is_default")]
    pub name: AnalogSensorName,

    #[serde(alias = "unit_of_meas", default, skip_serializing_if = "is_default")]
    pub unit_of_measurement: Option<String>,
}

impl AnalogSensor {
    pub fn new_with_state_topic<P: Into<String>>(state_topic: P) -> Self {
        Self {
            mqtt: MqttConfig::new_with_state_topic(state_topic),
            device_class: AnalogSensorClass::default(),
            name: AnalogSensorName::default(),
            unit_of_measurement: None,
        }
    }
}

impl From<&AnalogSensor> for Component {
    fn from(sensor: &AnalogSensor) -> Self {
        Self::Sensor
    }
}
