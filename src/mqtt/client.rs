// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::Context as _;
use bytes::{BufMut, BytesMut};
use futures::ready;
use pin_project::pin_project;
use rumqttc::{AsyncClient, LastWill, MqttOptions as RuMqttOptions, QoS};
use serde::{Deserialize, Serialize};
use tokio::task::{spawn, JoinHandle};
use tracing::{debug, warn};

use std::cell::RefCell;
use std::convert::TryFrom;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use super::home_assistant as hass;
use super::settings::MqttSettings;

#[pin_project]
#[derive(Debug, Deserialize)]
#[serde(try_from = "MqttSettings")]
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

    /// A `JoinHandle` for the task running the rumqttc event loop.
    #[pin]
    loop_task: JoinHandle<Result<(), anyhow::Error>>,
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

#[derive(Debug, Deserialize, Serialize)]
struct TopicState {
    /// The status of this device. Normally [Online], but the MQTT LWT should set it to [Offline]
    /// when we disconnect from the server.
    #[serde(default)]
    status: Status,

    /// The temperature of the camera, if available.
    #[serde(default)]
    temperature: Option<f32>,

    /// Whether or not the camera senses a person in its view.
    #[serde(default = "default_occupied")]
    occupied: bool,

    // TODO: Consider adding the coordinates of detected objects later on, but for now just skip
    // them.
    /// The number of objects detected.
    #[serde(default = "default_count")]
    count: u32,
    // TODO: Add last_update field
}

/// The status of a device as known to the MQTT server.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Online,
    Offline,
}

/// The length of the internal buffer of MQTT packets used by the `rumqttc` event loop.
const EVENT_LOOP_CAPACITY: usize = 10;

impl MqttClient {
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
            for address in address_iterator {
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

    async fn publish_discovery_config<'a, T: 'a>(&mut self, config: &'a T) -> anyhow::Result<()>
    where
        T: Serialize,
        hass::Component: From<&'a T>,
    {
        let topic = format!(
            "{}/{}/{}/config",
            self.home_assistant_topic,
            hass::Component::from(config).to_string(),
            self.device_uid
        );
        let mut payload = BytesMut::new().writer();
        serde_json::to_writer(&mut payload, config).context("serializing MQTT discovery config")?;
        self.client
            .publish_bytes(
                topic,
                QoS::AtLeastOnce,
                self.home_assistant_retain,
                payload.into_inner().freeze(),
            )
            .await?;
        Ok(())
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
        temperature_config.set_name(format!("{} Temperature", self.name).into());
        // TODO: let this be temperature_configurable?
        temperature_config.set_unit_of_measurement(Some("C".to_string()));
        temperature_config.set_unique_id(Some(self.unique_id_for(Topic::Temperature)));
        self.publish_discovery_config(&temperature_config).await?;

        let mut count_config = hass::AnalogSensor::new_with_state_topic_and_device(
            self.topic_for(Topic::Count),
            &device,
        );
        count_config.add_availability_topic(self.topic_for(Topic::Status));
        count_config.set_name(format!("{} Occupancy Count", self.name).into());
        count_config.set_unit_of_measurement(Some("people".to_string()));
        count_config.set_unique_id(Some(self.unique_id_for(Topic::Count)));
        self.publish_discovery_config(&count_config).await?;

        let mut occupancy_config = hass::BinarySensor::new_with_state_topic_and_device(
            self.topic_for(Topic::Occupancy),
            &device,
        );
        occupancy_config.add_availability_topic(self.topic_for(Topic::Status));
        occupancy_config.set_device_class(hass::BinarySensorClass::Occupancy);
        occupancy_config.set_name(format!("{} Occupancy", self.name).into());
        occupancy_config.set_unique_id(Some(self.unique_id_for(Topic::Occupancy)));
        self.publish_discovery_config(&occupancy_config).await?;

        Ok(())
    }
}

impl Future for MqttClient {
    type Output = Result<(), anyhow::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match ready!(self.project().loop_task.poll(cx)) {
            Ok(inner_result) => Poll::Ready(inner_result),
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }
}

impl TryFrom<MqttSettings> for MqttClient {
    type Error = anyhow::Error;

    fn try_from(settings: MqttSettings) -> Result<Self, Self::Error> {
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
        let (client, mut eventloop) = AsyncClient::new(client_options, EVENT_LOOP_CAPACITY);
        let loop_task = spawn(async move {
            loop {
                let event = eventloop
                    .poll()
                    .await
                    .context("polling the MQTT event loop")?;
                debug!("MQTT event processed: {:?}", event);
            }
        });
        // Extract values from settings
        let device_uid = settings.unique_id();
        let name = settings.name;
        Ok(Self {
            name,
            device_uid,
            home_assistant: settings.home_assistant,
            home_assistant_topic: settings.home_assistant_topic,
            home_assistant_retain: settings.home_assistant_retain,
            client,
            loop_task,
        })
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

fn default_occupied() -> bool {
    false
}

fn default_count() -> u32 {
    0
}
