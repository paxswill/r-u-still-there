// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{anyhow, Context as _};
use bytes::{BufMut, BytesMut};
use futures::{Sink, SinkExt, Stream, StreamExt};
use mqttbytes::{v4, QoS};
use pin_project::pin_project;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::webpki;
use tokio_rustls::{client::TlsStream, TlsConnector};
use tokio_util::codec::Framed;
use tracing::warn;

use std::cell::RefCell;
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use super::codec::{Error as MqttError, MqttCodec};
use super::home_assistant as hass;
use super::settings::MqttSettings;

type MqttFramed<V> = Framed<MqttStream, MqttCodec<V>>;
type SharedLazyStream<V> = Arc<Mutex<Option<MqttFramed<V>>>>;

#[derive(Debug)]
pub struct MqttClient {
    /// The various settings for connecting to the MQTT server.
    settings: MqttSettings,

    /// The current TCP connection to the MQTT server.
    // TODO: figure out how to better implement MQTT v4 and v5
    connection: SharedLazyStream<v4::Packet>,
}

#[pin_project(project = ProjectedMqttStream)]
#[derive(Debug)]
pub enum MqttStream {
    Unencrypted(#[pin] TcpStream),
    Encrypted(#[pin] TlsStream<TcpStream>),
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

/// The different topics that will be published.
#[derive(Clone, Copy)]
pub(crate) enum Topic {
    /// The status topic.
    Status,

    /// The temperature of the camera.
    Temperature,

    /// Whether or not the camera detects a person.
    Occupancy,

    /// How many people the camera is detecting.
    Count,

    /// The Home Assistant MQTT discovery topic for this device.
    Discovery(hass::Component),
}

async fn next_packet(
    framed: &mut MqttFramed<v4::Packet>,
) -> std::result::Result<v4::Packet, MqttError> {
    loop {
        match framed.next().await {
            None => (),
            Some(res) => return res,
        }
    }
}

impl MqttClient {
    pub async fn connect(&mut self) -> anyhow::Result<SharedLazyStream<v4::Packet>> {
        let mut possible_connection = self.connection.lock().unwrap();
        // Create the connection if needed.
        if possible_connection.is_none() {
            let url = self.settings.server_url();
            let host_str = url
                .host_str()
                .ok_or(anyhow!("MQTT URL somehow doesn't have a host"))?;
            let server_port = format!(
                "{}:{}",
                host_str,
                url.port().ok_or(anyhow!("Unset port for the MQTT URL"))?
            );
            // A TCP stream is needed for both encrypted and unencrypted connections, but the
            // encrypted connections is layered on top of it.
            let tcp_stream = TcpStream::connect(server_port).await?;
            let stream = if url.scheme() == "mqtts" {
                let mut tls_config = ClientConfig::new();
                // If disabling client verification was ever supported, it would be done here.
                // On second thought, provide a way to use a custom certificate as the trust root,
                // but not completely disable verification.
                tls_config
                    .root_store
                    .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
                let connector = TlsConnector::from(Arc::new(tls_config));
                let dns_name = webpki::DNSNameRef::try_from_ascii_str(host_str)?;
                let tls_stream = connector.connect(dns_name, tcp_stream).await?;
                MqttStream::Encrypted(tls_stream)
            } else if url.scheme() == "mqtt" {
                MqttStream::Unencrypted(tcp_stream)
            } else {
                panic!("The MQTT server scheme should've been restricted to 'mqtt' or 'mqtts'");
            };
            let mut framed = Framed::new(stream, MqttCodec::new());
            // Put the
            // At this point, we have an active stream to the broker. Now we let the broker know
            // we're connected.
            let connect_packet = v4::Packet::Connect(self.connect_data());
            framed
                .feed(connect_packet)
                .await
                .context("unable to send connect packet to broker")?;
            // The next packet should be a ConnAck
            let broker_packet = next_packet(&mut framed)
                .await
                .context("attempting to get the connection acknowledgement from the MQTT broker")?;
            // Confirm that the connection succeeded.
            if let v4::Packet::ConnAck(connack) = broker_packet {
                if connack.code != v4::ConnectReturnCode::Success {
                    // TODO: better error messages for the various return codes
                    return Err(anyhow!("Unable to connect to MQTT broker: {:?}", connack));
                }
            } else {
                return Err(anyhow!(
                    "Did not receive a connection acknowledgement from the broker: {:?}",
                    broker_packet
                ));
            }
            *possible_connection = Some(framed);
        }
        Ok(Arc::clone(&self.connection))
    }

    fn topic_for(&self, topic: Topic) -> String {
        // TODO: add a setting to override the base topic
        let topic_name = match topic {
            Topic::Status => "status",
            Topic::Temperature => "temperature",
            Topic::Occupancy => "occupied",
            Topic::Count => "count",
            // The discovery topic is special
            Topic::Discovery(component) => {
                return format!(
                    "{}/{}/{}/config",
                    self.settings.home_assistant_topic,
                    component.to_string(),
                    self.settings.unique_id()
                );
            }
        };
        format!("r_u_still_there/{}/{}", self.settings.name, topic_name)
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

    fn connect_data(&self) -> v4::Connect {
        let mut data: v4::Connect = (&self.settings).into();
        let payload = serde_json::to_vec(&Status::Offline)
            .expect("a static Status enum to encode cleanly into JSON");
        data.last_will = Some(v4::LastWill::new(
            self.topic_for(Topic::Status),
            payload,
            QoS::AtLeastOnce,
            true,
        ));
        data
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
        device.sw_version = option_env!("CARGO_PKG_VERSION").map(|vers| format!("r-u-still-there v{}", vers));
        Rc::new(RefCell::new(device))
    }

    async fn publish_discovery_config<'a, T: 'a>(
        &mut self,
        config: &'a T,
        framed: &mut MqttFramed<v4::Packet>,
    ) -> anyhow::Result<()>
    where
        T: Serialize,
        hass::Component: From<&'a T>,
    {
        let mut payload = BytesMut::new().writer();
        serde_json::to_writer(&mut payload, config)
            .context("serializing MQTT discovery config")?;
        let mut packet_data = v4::Publish::from_bytes(
            self.topic_for(Topic::Discovery(config.into())),
            // TODO: More QoS tracking
            QoS::AtMostOnce,
            payload.into_inner().freeze()
        );
        // Retain the configuration
        packet_data.retain = true;
        // TODO: implement tracking for QoS
        framed.feed(v4::Packet::Publish(packet_data)).await
            .context("sending discovery configuration to MQTT broker")?;
        Ok(())
    }

    async fn publish_home_assistant(&mut self, framed: &mut MqttFramed<v4::Packet>) -> anyhow::Result<()> {
        if !self.settings.home_assistant {
            warn!("Publishing Home Assistant discovery data without that option set");
        }
        let device = self.create_hass_device();

        let mut temperature_config = hass::AnalogSensor::new_with_state_topic_and_device(
            self.topic_for(Topic::Temperature),
            &device
        );
        temperature_config.add_availability_topic(self.topic_for(Topic::Status));
        temperature_config.set_device_class(hass::AnalogSensorClass::Temperature);
        temperature_config.set_name(format!("{} Temperature", self.settings.name).into());
        // TODO: let this be temperature_configurable?
        temperature_config.set_unit_of_measurement(Some("C".to_string()));
        temperature_config.set_unique_id(Some(self.unique_id_for(Topic::Temperature)));
        self.publish_discovery_config(&temperature_config, framed).await?;

        let mut count_config = hass::AnalogSensor::new_with_state_topic_and_device(
            self.topic_for(Topic::Count),
            &device
        );
        count_config.add_availability_topic(self.topic_for(Topic::Status));
        count_config.set_name(format!("{} Occupancy Count", self.settings.name).into());
        count_config.set_unit_of_measurement(Some("people".to_string()));
        count_config.set_unique_id(Some(self.unique_id_for(Topic::Count)));
        self.publish_discovery_config(&count_config, framed).await?;

        let mut occupancy_config = hass::BinarySensor::new_with_state_topic_and_device(
            self.topic_for(Topic::Occupancy),
            &device
        );
        occupancy_config.add_availability_topic(self.topic_for(Topic::Status));
        occupancy_config.set_device_class(hass::BinarySensorClass::Occupancy);
        occupancy_config.set_name(format!("{} Occupancy", self.settings.name).into());
        occupancy_config.set_unique_id(Some(self.unique_id_for(Topic::Occupancy)));
        self.publish_discovery_config(&occupancy_config, framed).await?;

        Ok(())
    }
}

impl From<MqttSettings> for MqttClient {
    fn from(settings: MqttSettings) -> Self {
        Self {
            settings,
            connection: Arc::new(Mutex::new(None)),
        }
    }
}

impl AsyncRead for MqttStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.project() {
            ProjectedMqttStream::Unencrypted(stream) => stream.poll_read(cx, buf),
            ProjectedMqttStream::Encrypted(stream) => stream.poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MqttStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match self.project() {
            ProjectedMqttStream::Unencrypted(stream) => stream.poll_write(cx, buf),
            ProjectedMqttStream::Encrypted(stream) => stream.poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        match self.project() {
            ProjectedMqttStream::Unencrypted(stream) => stream.poll_flush(cx),
            ProjectedMqttStream::Encrypted(stream) => stream.poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        match self.project() {
            ProjectedMqttStream::Unencrypted(stream) => stream.poll_shutdown(cx),
            ProjectedMqttStream::Encrypted(stream) => stream.poll_shutdown(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        match self.project() {
            ProjectedMqttStream::Unencrypted(stream) => stream.poll_write_vectored(cx, bufs),
            ProjectedMqttStream::Encrypted(stream) => stream.poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            MqttStream::Unencrypted(stream) => stream.is_write_vectored(),
            MqttStream::Encrypted(stream) => stream.is_write_vectored(),
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
