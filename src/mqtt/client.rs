// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::Context as _;
use bytes::{BufMut, BytesMut};
use pin_project::pin_project;
use rumqttc::{AsyncClient, EventLoop, LastWill, MqttOptions as RuMqttOptions, QoS};
use serde::{Deserialize, Serialize};
use tracing::warn;

use std::cell::RefCell;
use std::convert::TryFrom;
use std::rc::Rc;

use super::home_assistant as hass;
use super::settings::MqttSettings;
use super::state::State;

#[pin_project]
#[derive(Debug)]
pub struct MqttClient {
    /// A name for the base topic for this device.
    name: String,

    /// A persistent, unique identifier for this device.
    ///
    /// This value need to be unique across different devices, but also persistent over the life of
    /// the device. By default the systemd `machine-id` is used as a seed to generate an ID
    /// automatically, but there are some uses for manually specifying it (ex: migrating an
    /// existing setup to a new installation, or using a volatile system that regenerates its
    /// `machine-id` on every boot).
    device_uid: String,

    /// Enable Home Assistant integration.
    ///
    /// When enabled, entities will be automatically added to Home Assistant using MQTT discovery.
    /// Do note that the MJPEG stream is *not* able to be automatically added in this way, you will
    /// need to add it manually.
    home_assistant: bool,

    /// The topic prefix used for Home Assistant MQTT discovery.
    ///
    /// Defaults to "homeassistant"
    home_assistant_topic: String,

    /// Retain Home Assistant MQTT discovery configuration on the MQTT broker.
    ///
    /// **In almost all cases this option should be enabled, and the default is to be enabled.**
    ///
    /// By disabling this, the entity configuration will not be stored on the MQTT broker, and Home
    /// Assistant will only receive it when r-u-still-there starts up.
    home_assistant_retain: bool,

    /// The MQTT client.
    client: AsyncClient,

    /// The state of this device (online or offline).
    status: State<Status>,

    /// The temperature as detected by the camera
    temperature: State<Option<f32>>,

    /// Whether or not the camera detects a person.
    occupied: State<bool>,

    /// The number of people the camera detects.
    count: State<u32>,

    // TODO: Consider adding last_update field, as well as adding the coordinates of all detected objects.
}

/// The different topics that will be published.
#[derive(Clone, Copy)]
pub enum Topic {
    /// The status topic.
    Status,

    /// The temperature of the camera.
    Temperature,

    /// Whether or not the camera detects a person.
    Occupancy,

    /// How many people the camera is detecting.
    Count,
}

/// The status of a device as known to the MQTT server.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Online,
    Offline,
}

/// The length of the internal buffer of MQTT packets used by the `rumqttc` event loop.
const EVENT_LOOP_CAPACITY: usize = 10;

impl MqttClient {
    pub fn connect(settings: MqttSettings) -> anyhow::Result<(Self, EventLoop)> {
        let device_uid = settings.unique_id();
        // Create rumqttc client and event loop task
        let mut client_options = RuMqttOptions::try_from(&settings)?;
        let payload = serde_json::to_vec(&Status::Offline)
            .expect("a static Status enum to encode cleanly into JSON");
        // TODO: add a setting to override the base topic
        let base_topic = "r-u-still-there";
        let status_topic = [base_topic, &device_uid, "status"].join("/");
        client_options.set_last_will(LastWill::new(
            &status_topic,
            payload,
            QoS::AtLeastOnce,
            true,
        ));
        let (client, eventloop) = AsyncClient::new(client_options, EVENT_LOOP_CAPACITY);
        // Create the states first, as they use a reference to device_uid
        let status = State::new_default_at(status_topic);
        let temperature = State::new_default_at([base_topic, &device_uid, "temperature"].join("/"));
        let occupied = State::new_default_at([base_topic, &device_uid, "occupied"].join("/"));
        let count = State::new_default_at([base_topic, &device_uid, "occupancy_count"].join("/"));
        Ok((
            Self {
                name: settings.name,
                device_uid,
                home_assistant: settings.home_assistant,
                home_assistant_topic: settings.home_assistant_topic,
                home_assistant_retain: settings.home_assistant_retain,
                client,
                status,
                temperature,
                occupied,
                count,
            },
            eventloop,
        ))
    }

    /// Publish the initial messages to the MQTT broker.
    ///
    /// These include Home Assistant discovery (if enabled) as well as the default values for the
    /// different states (temperature, occupancy, etc).
    pub async fn publish_initial(&mut self) -> anyhow::Result<()> {
        if self.home_assistant {
            self.publish_home_assistant().await?;
        }
        self.status.publish(&mut self.client).await?;
        self.temperature.publish(&mut self.client).await?;
        self.occupied.publish(&mut self.client).await?;
        self.count.publish(&mut self.client).await?;
        Ok(())
    }

    fn unique_id_for(&self, topic: Topic) -> String {
        let entity_kind = match topic {
            // Status doesn't really need a unique_id, buyt just in case I add it later
            Topic::Status => "status",
            Topic::Temperature => "temperature",
            Topic::Occupancy => "occupied",
            Topic::Count => "count",
        };
        format!("{}_{}_r-u-still-there", self.device_uid, entity_kind)
    }
    /// Create the device description common to all entities in Home Assistant.
    fn create_hass_device(&self) -> Rc<RefCell<hass::Device>> {
        let mut device = hass::Device::default();
        // Add all the MAC addresses to our device, it'll update whatever Home Assistant has.
        let mac_addresses = match mac_address::MacAddressIterator::new() {
            Ok(address_iterator) => Some(address_iterator),
            Err(e) => {
                warn!("unable to access MAC addresses: {:?}", e);
                None
            }
        };
        if let Some(address_iterator) = mac_addresses {
            // Filter out all-zero MAC addresses (like from a loopback interface)
            let filtered_addresses = address_iterator.filter(|a| a.bytes() != [0u8; 6]);
            for address in filtered_addresses {
                device.add_mac_connection(address);
            }
        }
        device.name = Some(self.name.clone());
        device.add_identifier(self.device_uid.clone());
        // TODO: investigate using the 'built' crate to also get Git hash
        device.sw_version =
            option_env!("CARGO_PKG_VERSION").map(|vers| format!("r-u-still-there v{}", vers));
        Rc::new(RefCell::new(device))
    }

    async fn publish_serialize<T>(
        &mut self,
        topic: String,
        qos: QoS,
        retain: bool,
        payload: &T,
    ) -> anyhow::Result<()>
    where
        T: Serialize,
    {
        let mut payload_data = BytesMut::new().writer();
        serde_json::to_writer(&mut payload_data, payload)?;
        self.client
            .publish_bytes(topic, qos, retain, payload_data.into_inner().freeze())
            .await?;
        Ok(())
    }

    async fn publish_discovery_config<'a, T: 'a>(
        &mut self,
        unique_id: &'a str,
        config: &'a T,
    ) -> anyhow::Result<()>
    where
        T: Serialize,
        hass::Component: From<&'a T>,
    {
        let topic = format!(
            "{}/{}/{}/config",
            self.home_assistant_topic,
            hass::Component::from(config).to_string(),
            unique_id
        );
        self.publish_serialize(topic, QoS::AtLeastOnce, self.home_assistant_retain, config)
            .await
            .context("serializing MQTT discovery config")
    }

    pub async fn publish_home_assistant(&mut self) -> anyhow::Result<()> {
        if !self.home_assistant_retain {
            warn!("Publishing Home Assistant discovery data without the retain flag");
        }
        let device = self.create_hass_device();

        let mut temperature_config = hass::AnalogSensor::new_with_state_topic_and_device(
            self.temperature.topic(),
            &device,
        );
        temperature_config.add_availability_topic(self.status.topic().into());
        temperature_config.set_device_class(hass::AnalogSensorClass::Temperature);
        temperature_config.set_name(format!("{} Temperature", self.name));
        // TODO: let this be temperature_configurable?
        temperature_config.set_unit_of_measurement(Some("C".to_string()));
        temperature_config.set_unique_id(Some(self.unique_id_for(Topic::Temperature)));
        self.publish_discovery_config(
            &temperature_config
                .unique_id()
                .as_ref()
                .expect("the unique ID to be what it was just set to"),
            &temperature_config,
        )
        .await?;

        let mut count_config = hass::AnalogSensor::new_with_state_topic_and_device(
            self.count.topic(),
            &device,
        );
        count_config.add_availability_topic(self.status.topic().into());
        count_config.set_name(format!("{} Occupancy Count", self.name));
        count_config.set_unit_of_measurement(Some("people".to_string()));
        count_config.set_unique_id(Some(self.unique_id_for(Topic::Count)));
        self.publish_discovery_config(
            &count_config
                .unique_id()
                .as_ref()
                .expect("the unique ID to be what it was just set to"),
            &count_config,
        )
        .await?;

        let mut occupancy_config = hass::BinarySensor::new_with_state_topic_and_device(
            self.occupied.topic(),
            &device,
        );
        occupancy_config.add_availability_topic(self.status.topic().into());
        occupancy_config.set_device_class(hass::BinarySensorClass::Occupancy);
        occupancy_config.set_name(format!("{} Occupancy", self.name));
        occupancy_config.set_unique_id(Some(self.unique_id_for(Topic::Occupancy)));
        self.publish_discovery_config(
            &occupancy_config
                .unique_id()
                .as_ref()
                .expect("the unique ID to be what it was just set to"),
            &occupancy_config,
        )
        .await?;

        Ok(())
    }

    /// Update the 'online' status of the device as published to the broker.
    ///
    /// If the new status is unchanged from the current status, no message is sent as the status is
    /// a retained message.
    pub async fn update_online(&mut self, online: bool) -> anyhow::Result<bool> {
        let new_status = Status::from(online);
        self.status.publish_if_update(new_status, &mut self.client).await
    }

    pub async fn update_temperature(&mut self, temperature: Option<f32>) -> anyhow::Result<bool> {
        self.temperature.publish_if_update(temperature, &mut self.client).await
    }

    pub async fn update_occupancy_count(&mut self, count: u32) -> anyhow::Result<bool> {
        self.occupied.publish_if_update(count == 0, &mut self.client).await?;
        self.count.publish_if_update(count, &mut self.client).await
    }
}

impl Default for Status {
    fn default() -> Self {
        Self::Online
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
