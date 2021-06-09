// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{anyhow, Context as _};
use futures::{Sink, SinkExt, Stream, StreamExt};
use mqttbytes::v4;
use pin_project::pin_project;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::webpki;
use tokio_rustls::{client::TlsStream, TlsConnector};
use tokio_util::codec::Framed;

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use super::codec::{Error as MqttError, MqttCodec};
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
            let connect_packet = v4::Packet::Connect((&self.settings).into());
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
