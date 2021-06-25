// SPDX-License-Identifier: GPL-3.0-or-later
use std::borrow::Borrow;
use std::string::ToString;

use serde::{Deserialize, Serialize};

use super::home_assistant as hass;
use super::state::DiscoveryValue;

/// The status of a device as known to the MQTT server.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Online,
    Offline,
}

impl Default for Status {
    fn default() -> Self {
        Self::Online
    }
}

impl<D> DiscoveryValue<D> for Status
where
    D: Borrow<hass::Device>,
    D: Default + PartialEq,
    D: Serialize,
{
    type Config = hass::BinarySensor<D>;

    fn retained() -> bool {
        true
    }

    fn component_type() -> hass::Component {
        hass::Component::BinarySensor
    }

    fn home_assistant_config(
        device: D,
        state_topic: String,
        availability_topic: String,
        name: String,
        unique_id: String,
    ) -> Self::Config {
        let mut config = hass::BinarySensor::new_with_state_topic_and_device(state_topic, device);
        config.add_availability_topic(availability_topic);
        config.set_name(name);
        config.set_unique_id(Some(unique_id));
        config.set_payload_on(Self::Online.to_string().into());
        config.set_payload_off(Self::Offline.to_string().into());
        config
    }
}

impl From<bool> for Status {
    fn from(online: bool) -> Self {
        if online {
            Self::Online
        } else {
            Self::Offline
        }
    }
}

impl ToString for Status {
    fn to_string(&self) -> String {
        match self {
            Status::Online => "online".to_string(),
            Status::Offline => "offline".to_string(),
        }
    }
}

/// The occupancy status of a location.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Occupancy {
    Occupied,
    Unoccupied,
}

impl Default for Occupancy {
    fn default() -> Self {
        Self::Unoccupied
    }
}

impl<D> DiscoveryValue<D> for Occupancy
where
    D: Borrow<hass::Device>,
    D: Default + PartialEq,
    D: Serialize,
{
    type Config = hass::BinarySensor<D>;

    fn retained() -> bool {
        true
    }

    fn component_type() -> hass::Component {
        hass::Component::BinarySensor
    }

    fn home_assistant_config(
        device: D,
        state_topic: String,
        availability_topic: String,
        name: String,
        unique_id: String,
    ) -> Self::Config {
        let mut config = hass::BinarySensor::new_with_state_topic_and_device(state_topic, device);
        config.add_availability_topic(availability_topic);
        config.set_device_class(hass::BinarySensorClass::Occupancy);
        config.set_name(name);
        config.set_unique_id(Some(unique_id));
        config.set_payload_on(Self::Occupied.to_string().into());
        config.set_payload_off(Self::Unoccupied.to_string().into());
        config
    }
}

impl From<bool> for Occupancy {
    fn from(occupied: bool) -> Self {
        if occupied {
            Self::Occupied
        } else {
            Self::Unoccupied
        }
    }
}

impl From<usize> for Occupancy {
    fn from(count: usize) -> Self {
        if count > 0 {
            Self::Occupied
        } else {
            Self::Unoccupied
        }
    }
}

impl From<OccupancyCount> for Occupancy {
    fn from(count: OccupancyCount) -> Self {
        Occupancy::from(count.0)
    }
}

impl ToString for Occupancy {
    fn to_string(&self) -> String {
        match self {
            Occupancy::Occupied => "occupied".to_string(),
            Occupancy::Unoccupied => "unoccupied".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OccupancyCount(usize);

impl Default for OccupancyCount {
    fn default() -> Self {
        Self(0)
    }
}

impl<D> DiscoveryValue<D> for OccupancyCount
where
    D: Borrow<hass::Device>,
    D: Default + PartialEq,
    D: Serialize,
{
    type Config = hass::AnalogSensor<D>;

    fn retained() -> bool {
        true
    }

    fn component_type() -> hass::Component {
        hass::Component::Sensor
    }

    fn home_assistant_config(
        device: D,
        state_topic: String,
        availability_topic: String,
        name: String,
        unique_id: String,
    ) -> Self::Config {
        let mut config = hass::AnalogSensor::new_with_state_topic_and_device(state_topic, device);
        config.add_availability_topic(availability_topic);
        config.set_unit_of_measurement(Some("people".to_string()));
        config.set_name(name);
        config.set_unique_id(Some(unique_id));
        config
    }
}

impl From<usize> for OccupancyCount {
    fn from(inner: usize) -> Self {
        Self(inner)
    }
}

impl From<OccupancyCount> for usize {
    fn from(outer: OccupancyCount) -> Self {
        outer.0
    }
}

// Fallback implementations for primitives

impl<D> DiscoveryValue<D> for bool
where
    D: Borrow<hass::Device>,
    D: Default + PartialEq,
    D: Serialize,
{
    type Config = hass::BinarySensor<D>;

    fn retained() -> bool {
        true
    }

    fn component_type() -> hass::Component {
        hass::Component::BinarySensor
    }

    fn home_assistant_config(
        device: D,
        state_topic: String,
        availability_topic: String,
        name: String,
        unique_id: String,
    ) -> Self::Config {
        let mut config = hass::BinarySensor::new_with_state_topic_and_device(state_topic, device);
        config.add_availability_topic(availability_topic);
        config.set_name(name);
        config.set_unique_id(Some(unique_id));
        config
    }
}

macro_rules! primitive_discovery_value {
    ($typ:ty) => {
        impl<D> DiscoveryValue<D> for $typ
        where
            D: Borrow<hass::Device>,
            D: Default + PartialEq,
            D: Serialize,
        {
            type Config = hass::AnalogSensor<D>;

            fn retained() -> bool {
                true
            }

            fn component_type() -> hass::Component {
                hass::Component::Sensor
            }

            fn home_assistant_config(
                device: D,
                state_topic: String,
                availability_topic: String,
                name: String,
                unique_id: String,
            ) -> Self::Config {
                let mut config =
                    hass::AnalogSensor::new_with_state_topic_and_device(state_topic, device);
                config.add_availability_topic(availability_topic);
                config.set_name(name);
                config.set_unique_id(Some(unique_id));
                config
            }
        }
    };
}

primitive_discovery_value!(f32);
primitive_discovery_value!(f64);
primitive_discovery_value!(usize);
primitive_discovery_value!(u8);
primitive_discovery_value!(u16);
primitive_discovery_value!(u32);
primitive_discovery_value!(u64);
primitive_discovery_value!(isize);
primitive_discovery_value!(i8);
primitive_discovery_value!(i16);
primitive_discovery_value!(i32);
primitive_discovery_value!(i64);
