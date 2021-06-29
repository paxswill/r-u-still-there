// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{anyhow, Context as _};
use futures::{ready, Future};
use rumqttc::{AsyncClient, EventLoop, LastWill, MqttOptions as RuMqttOptions, QoS};
use serde::Serialize;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;

use std::convert::TryFrom;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use super::home_assistant as hass;
use super::serialize::serialize;
use super::settings::MqttSettings;
use super::state::State;
use super::state_values::{Occupancy, OccupancyCount, Status};

#[derive(Clone, Copy, Debug)]
pub enum ClientMessage {
    UpdateTemperature(Option<f32>),
    UpdateOccupancy(usize),
    UpdateStatus(bool),
}

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

    /// The MQTT client.
    client: Arc<Mutex<AsyncClient>>,

    /// The state of this device (online or offline).
    status: State<Status, hass::Device>,

    /// The temperature as detected by the camera
    temperature: State<f32, hass::Device>,

    /// Whether or not the camera detects a person.
    occupied: State<Occupancy, hass::Device>,

    /// The number of people the camera detects.
    count: State<OccupancyCount, hass::Device>,

    // TODO: Consider adding last_update field, as well as adding the coordinates of all detected objects.
    /// Send side of a channel used to send commands to the client while it's running. Kept so that
    /// It can be freely cloned and to ensure the receiver side stays open.
    command_tx: mpsc::Sender<ClientMessage>,

    /// Receive side of a channel used to send commands to this client while it's running.
    command_rx: mpsc::Receiver<ClientMessage>,

    /// A [Future] for updating one of the states.
    in_progress_future: Option<Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>>>,
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

/// The length of the internal buffer of MQTT packets used by the `rumqttc` event loop.
const EVENT_LOOP_CAPACITY: usize = 20;

impl MqttClient {
    pub fn connect(settings: MqttSettings) -> anyhow::Result<(Self, EventLoop)> {
        let device_uid = settings.unique_id();
        // Create rumqttc client and event loop task
        let mut client_options = RuMqttOptions::try_from(&settings)?;
        // TODO: add a setting to override the base topic
        let base_topic = "r-u-still-there";
        let device_name = settings.name;
        let status_topic = [base_topic, &device_uid, "status"].join("/");
        client_options.set_last_will(LastWill::new(
            &status_topic,
            Status::Offline.to_string().as_bytes(),
            QoS::AtLeastOnce,
            true,
        ));
        let (client, eventloop) = AsyncClient::new(client_options, EVENT_LOOP_CAPACITY);
        let client = Arc::new(Mutex::new(client));
        let (command_tx, command_rx) = mpsc::channel(30);
        // Create the states early, as they use a reference to device_uid
        let status = State::new(
            Arc::clone(&client),
            base_topic,
            &device_name,
            "status",
            true,
            QoS::AtLeastOnce,
        );
        let temperature = State::new(
            Arc::clone(&client),
            base_topic,
            &device_name,
            "temperature",
            true,
            QoS::AtLeastOnce,
        );
        let occupied = State::new(
            Arc::clone(&client),
            base_topic,
            &device_name,
            "occupied",
            true,
            QoS::AtLeastOnce,
        );
        let count = State::new(
            Arc::clone(&client),
            base_topic,
            &device_name,
            "occupancy_count",
            true,
            QoS::AtLeastOnce,
        );
        Ok((
            Self {
                name: device_name,
                device_uid,
                home_assistant: settings.home_assistant,
                home_assistant_topic: settings.home_assistant_topic,
                client,
                status,
                temperature,
                occupied,
                count,
                command_tx,
                command_rx,
                in_progress_future: None,
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
        // Only keep the lock as long as required for each client.
        self.status.publish().await?;
        self.temperature.publish().await?;
        self.occupied.publish().await?;
        self.count.publish().await?;
        Ok(())
    }

    fn unique_id_for(&self, topic: Topic) -> String {
        let entity_kind = match topic {
            // Status doesn't really need a unique_id, but just in case I add it later
            Topic::Status => "status",
            Topic::Temperature => "temperature",
            Topic::Occupancy => "occupied",
            Topic::Count => "count",
        };
        format!("{}_{}_r-u-still-there", self.device_uid, entity_kind)
    }
    /// Create the device description common to all entities in Home Assistant.
    fn create_hass_device(&self) -> Arc<hass::Device> {
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
        Arc::new(device)
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
        let payload_data = serialize(payload)?;
        self.client
            .lock()
            .await
            .publish(topic, qos, retain, payload_data)
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
        // Always retain discovery messages.
        self.publish_serialize(topic, QoS::AtLeastOnce, true, config)
            .await
            .context("serializing MQTT discovery config")
    }

    pub async fn publish_home_assistant(&mut self) -> anyhow::Result<()> {
        let device = self.create_hass_device();

        let mut temperature_config = hass::AnalogSensor::new_with_state_topic_and_device(
            self.temperature.topic(),
            Arc::clone(&device),
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
            Arc::clone(&device),
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
            Arc::clone(&device),
        );
        occupancy_config.add_availability_topic(self.status.topic().into());
        occupancy_config.set_device_class(hass::BinarySensorClass::Occupancy);
        occupancy_config.set_name(format!("{} Occupancy", self.name));
        occupancy_config.set_unique_id(Some(self.unique_id_for(Topic::Occupancy)));
        occupancy_config.set_payload_on(Occupancy::Occupied.to_string().into());
        occupancy_config.set_payload_off(Occupancy::Unoccupied.to_string().into());
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

    pub fn command_channel(&self) -> mpsc::Sender<ClientMessage> {
        self.command_tx.clone()
    }
}

impl Future for MqttClient {
    type Output = anyhow::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            if let Some(fut) = &mut self.in_progress_future {
                match ready!(fut.as_mut().poll(cx)) {
                    Err(e) => {
                        return Poll::Ready(Err(e));
                    }
                    Ok(_) => {
                        self.in_progress_future = None;
                    }
                }
            }
            match ready!(self.command_rx.poll_recv(cx)) {
                Some(msg) => match msg {
                    ClientMessage::UpdateTemperature(temperature) => {
                        let state = self.temperature.clone();
                        self.in_progress_future = Some(Box::pin(async move {
                            state.publish_if_update(temperature.unwrap_or(0.0)).await
                        }));
                    }
                    ClientMessage::UpdateOccupancy(count) => {
                        let binary_state = self.occupied.clone();
                        let count_state = self.count.clone();
                        self.in_progress_future = Some(Box::pin(async move {
                            binary_state.publish_if_update(count.into()).await?;
                            count_state.publish_if_update(count.into()).await
                        }));
                    }
                    ClientMessage::UpdateStatus(status) => {
                        let new_status = Status::from(status);
                        let state = self.status.clone();
                        self.in_progress_future = Some(Box::pin(async move {
                            state.publish_if_update(new_status).await
                        }));
                    }
                },
                None => {
                    return Poll::Ready(Err(anyhow!("Client command channel closed unexpectedly")))
                }
            }
        }
    }
}
