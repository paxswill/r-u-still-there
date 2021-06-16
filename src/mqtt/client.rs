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

    /// The current state of the various outputs.
    state: TopicState,
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
struct TopicState {
    /// The status of this device. Normally [Online], but the MQTT LWT should set it to [Offline]
    /// when we disconnect from the server.
    #[serde(default)]
    status: Status,

    /// The temperature of the camera, if available.
    #[serde(default)]
    temperature: Option<f32>,

    /// Whether or not the camera senses a person in its view.
    #[serde(default = "TopicState::default_occupied")]
    occupied: bool,

    // TODO: Consider adding the coordinates of detected objects later on, but for now just skip
    // them.
    /// The number of objects detected.
    #[serde(default = "TopicState::default_count")]
    count: u32,
    // TODO: Add last_update field
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
        // Create rumqttc client and event loop task
        let mut client_options = RuMqttOptions::try_from(&settings)?;
        let payload = serde_json::to_vec(&Status::Offline)
            .expect("a static Status enum to encode cleanly into JSON");
        client_options.set_last_will(LastWill::new(
            topic_for(&settings.name, Topic::Status),
            payload,
            QoS::AtLeastOnce,
            true,
        ));
        let (client, eventloop) = AsyncClient::new(client_options, EVENT_LOOP_CAPACITY);
        // Extract values from settings
        let device_uid = settings.unique_id();
        let name = settings.name;
        Ok((
            Self {
                name,
                device_uid,
                home_assistant: settings.home_assistant,
                home_assistant_topic: settings.home_assistant_topic,
                home_assistant_retain: settings.home_assistant_retain,
                client,
                state: TopicState::default(),
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
        // TODO: This code duplication is screaming out for a better approach.
        // Status
        let status = self.state.status;
        self.publish_serialize(
            self.topic_for(Topic::Status),
            QoS::AtLeastOnce,
            true,
            &status,
        )
        .await?;
        // Temperature
        let temperature = self.state.temperature;
        self.publish_serialize(
            self.topic_for(Topic::Temperature),
            QoS::AtLeastOnce,
            true,
            &temperature,
        )
        .await?;
        // Occupancy
        let occupied = self.state.occupied;
        self.publish_serialize(
            self.topic_for(Topic::Occupancy),
            QoS::AtLeastOnce,
            true,
            &occupied,
        )
        .await?;
        // Occupancy count
        let count = self.state.count;
        self.publish_serialize(
            self.topic_for(Topic::Count),
            QoS::AtLeastOnce,
            true,
            &count,
        )
        .await?;

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

    /// Generate the full topic that a type of value whould be published at.
    ///
    /// This just forwards to the free function `topic_for`, providing the static values from `self`.
    fn topic_for(&self, topic: Topic) -> String {
        topic_for(&self.name, topic)
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
            self.topic_for(Topic::Temperature),
            &device,
        );
        temperature_config.add_availability_topic(self.topic_for(Topic::Status));
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
            self.topic_for(Topic::Count),
            &device,
        );
        count_config.add_availability_topic(self.topic_for(Topic::Status));
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
            self.topic_for(Topic::Occupancy),
            &device,
        );
        occupancy_config.add_availability_topic(self.topic_for(Topic::Status));
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
    pub async fn update_online(&mut self, online: bool) -> anyhow::Result<()> {
        let new_status = Status::from(online);
        if self.state.status == new_status {
            debug!(status = ?new_status, "Skipping unchanged status update");
            return Ok(());
        }
        self.publish_serialize(
            self.topic_for(Topic::Status),
            QoS::AtLeastOnce,
            true,
            &new_status,
        )
        .await
    }
}

/// Generate the full topic given a topic type.
// This used to be a member function of MqttClient, but it's also needed when creating the Last
// Will message, so it was pulled out into a free function.
fn topic_for(name: &str, topic: Topic) -> String {
    // TODO: add a setting to override the base topic
    let topic_name = match topic {
        Topic::Status => "status",
        Topic::Temperature => "temperature",
        Topic::Occupancy => "occupied",
        Topic::Count => "count",
    };
    format!("r_u_still_there/{}/{}", name, topic_name)
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

impl TopicState {
    fn default_occupied() -> bool {
        false
    }

    fn default_count() -> u32 {
        0
    }
}

impl Default for TopicState {
    fn default() -> Self {
        Self {
            status: Status::default(),
            temperature: None,
            occupied: false,
            count: 0,
        }
    }
}
