// SPDX-License-Identifier: GPL-3.0-or-later
use std::cell::Ref;
use std::collections::HashSet;

use paste::paste;
use serde::{Deserialize, Serialize};

use super::{
    is_default, AvailabilityMode, AvailabilityTopic, ForceUpdate, MqttConfig, PayloadAvailable,
    PayloadNotAvailable, SensorQoS,
};
use crate::{default_newtype, default_string};

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
