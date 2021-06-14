// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::Context as _;
use bytes::{BufMut, BytesMut};
use futures::ready;
use pin_project::pin_project;
use rumqttc::{AsyncClient, LastWill, MqttOptions as RuMqttOptions, QoS};
use serde::{Deserialize, Serialize};
use tokio::task::{JoinHandle, spawn};
use tracing::{debug, warn};

use std::cell::RefCell;
use std::convert::TryFrom;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use super::home_assistant as hass;
use super::settings::{MqttSettings, Topic};

#[pin_project]
#[derive(Debug)]
pub struct MqttClient {
    /// The various settings for connecting to the MQTT server.
    settings: MqttSettings,

    /// The MQTT client.
    client: AsyncClient,

    /// A `JoinHandle` for the task running the rumqttc event loop.
    #[pin]
    loop_task: JoinHandle<Result<(), anyhow::Error>>,
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

// Just using the value in the rumqttc example. Not even sure what it does :/
const EVENT_LOOP_CAPACITY: usize = 10;

impl MqttClient {
    pub fn new(settings: MqttSettings) -> anyhow::Result<Self> {
        let mut client_options = RuMqttOptions::try_from(&settings)?;
        let payload = serde_json::to_vec(&Status::Offline)
            .expect("a static Status enum to encode cleanly into JSON");
        client_options.set_last_will(LastWill::new(
            settings.topic_for(Topic::Status),
            payload,
            QoS::AtLeastOnce,
            true,
        ));
        let (client, mut eventloop) = AsyncClient::new(client_options, EVENT_LOOP_CAPACITY);
        let loop_task = spawn(async move {
            loop {
                let event = eventloop.poll().await.context("polling the MQTT event loop")?;
                debug!("MQTT event processed: {:?}", event);
            }
        });
        Ok(Self {
            settings,
            client,
            loop_task,
        })
    }

    fn unique_id_for(&self, topic: Topic) -> String {
        let unique_id = self.settings.unique_id();
        let entity_kind = match topic {
            // Status doesn't really need a unique_id, buyt just in case I add it later
            Topic::Status => "status",
            Topic::Temperature => "temperature",
            Topic::Occupancy => "occupied",
            Topic::Count => "count",
            // For "discovery" just use the bare device unique ID.
            Topic::Discovery(_) => return unique_id,
        };
        format!("{}_{}_r-u-still-there", unique_id, entity_kind)
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
        device.name = Some(self.settings.name.clone());
        device.add_identifier(self.settings.unique_id());
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
        let mut payload = BytesMut::new().writer();
        serde_json::to_writer(&mut payload, config).context("serializing MQTT discovery config")?;
        let topic = self
            .client
            .publish_bytes(
                self.settings.topic_for(Topic::Discovery(config.into())),
                QoS::AtLeastOnce,
                // Retain the config data
                true,
                payload.into_inner().freeze(),
            )
            .await?;
        Ok(())
    }

    pub async fn publish_home_assistant(&mut self) -> anyhow::Result<()> {
        if !self.settings.home_assistant {
            warn!("Publishing Home Assistant discovery data without that option set");
        }
        let device = self.create_hass_device();

        let mut temperature_config = hass::AnalogSensor::new_with_state_topic_and_device(
            self.settings.topic_for(Topic::Temperature),
            &device,
        );
        temperature_config.add_availability_topic(self.settings.topic_for(Topic::Status));
        temperature_config.set_device_class(hass::AnalogSensorClass::Temperature);
        temperature_config.set_name(format!("{} Temperature", self.settings.name).into());
        // TODO: let this be temperature_configurable?
        temperature_config.set_unit_of_measurement(Some("C".to_string()));
        temperature_config.set_unique_id(Some(self.unique_id_for(Topic::Temperature)));
        self.publish_discovery_config(&temperature_config).await?;

        let mut count_config = hass::AnalogSensor::new_with_state_topic_and_device(
            self.settings.topic_for(Topic::Count),
            &device,
        );
        count_config.add_availability_topic(self.settings.topic_for(Topic::Status));
        count_config.set_name(format!("{} Occupancy Count", self.settings.name).into());
        count_config.set_unit_of_measurement(Some("people".to_string()));
        count_config.set_unique_id(Some(self.unique_id_for(Topic::Count)));
        self.publish_discovery_config(&count_config).await?;

        let mut occupancy_config = hass::BinarySensor::new_with_state_topic_and_device(
            self.settings.topic_for(Topic::Occupancy),
            &device,
        );
        occupancy_config.add_availability_topic(self.settings.topic_for(Topic::Status));
        occupancy_config.set_device_class(hass::BinarySensorClass::Occupancy);
        occupancy_config.set_name(format!("{} Occupancy", self.settings.name).into());
        occupancy_config.set_unique_id(Some(self.unique_id_for(Topic::Occupancy)));
        self.publish_discovery_config(&occupancy_config).await?;

        Ok(())
    }
}

impl Future for MqttClient {
    type Output = Result<(), anyhow::Error>;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        match ready!(self.project().loop_task.poll(cx)) {
            Ok(inner_result) => Poll::Ready(inner_result),
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }
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
