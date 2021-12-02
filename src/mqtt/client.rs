// SPDX-License-Identifier: GPL-3.0-or-later

use std::convert::TryFrom;
use std::error::Error;
use std::io::ErrorKind;

use anyhow::{anyhow, Context as _};
use rumqttc::{
    AsyncClient, ConnectReturnCode, Event, EventLoop, LastWill, MqttOptions as RuMqttOptions,
    Packet, QoS,
};
use serde::Serialize;
use tokio::sync::watch;
use tracing::{debug, error, trace, warn};

use crate::mqtt::Status;

use super::serialize::serialize;
use super::MqttSettings;

#[derive(Clone, Debug)]
pub(crate) struct MqttSender {
    sender: rumqttc::Sender<rumqttc::Request>,
    connected: watch::Receiver<bool>,
}

impl MqttSender {
    pub(crate) async fn enqueue_publish<T: Serialize>(
        &mut self,
        topic: String,
        qos: QoS,
        payload: &T,
        retain: bool,
    ) -> anyhow::Result<()> {
        trace!("Enqueuing MQTT publish");
        let payload = serialize(payload)?;
        let mut message = rumqttc::Publish::new(topic, qos, payload);
        message.retain = retain;
        self.sender
            .send(message.into())
            .await
            .context("Sending publish message to internal MQTT client")?;
        Ok(())
    }

    pub(crate) async fn publish_if_connected<T: Serialize>(
        &mut self,
        topic: String,
        qos: QoS,
        payload: &T,
        retain: bool,
    ) -> anyhow::Result<bool> {
        // Block until we're connected
        let connected = *self.connected.borrow_and_update();
        if connected {
            self.enqueue_publish(topic, qos, payload, retain).await?;
        }
        Ok(connected)
    }

    pub(crate) async fn publish_when_connected<T: Serialize>(
        &mut self,
        topic: String,
        qos: QoS,
        payload: &T,
        retain: bool,
    ) -> anyhow::Result<()> {
        // Block until we're connected
        let mut connected = *self.connected.borrow_and_update();
        while !connected {
            self.connected
                .changed()
                .await
                .context("Waiting for internal MQTT client to connect")?;
            connected = *self.connected.borrow_and_update();
        }
        self.enqueue_publish(topic, qos, payload, retain).await
    }
}

pub(crate) struct MqttClient {
    status_topic: String,
    event_loop: EventLoop,
    connected: watch::Sender<bool>,
    sender: rumqttc::Sender<rumqttc::Request>,
}

impl MqttClient {
    const EVENT_LOOP_CAPACITY: usize = 20;

    pub(crate) fn new(settings: &MqttSettings) -> anyhow::Result<Self> {
        let status_topic = [&settings.base_topic, &settings.name, "status"].join("/");
        let mut client_options = RuMqttOptions::try_from(settings)?;
        client_options
            .set_last_will(LastWill::new(
                &status_topic,
                Status::Offline.to_string().as_bytes(),
                QoS::AtLeastOnce,
                true,
            ))
            .set_connection_timeout(10);
        let (connected, _) = watch::channel(false);
        let (_, event_loop) = AsyncClient::new(client_options, Self::EVENT_LOOP_CAPACITY);
        let sender = event_loop.handle();
        Ok(Self {
            status_topic,
            event_loop,
            connected,
            sender,
        })
    }

    pub(crate) fn status_topic(&self) -> &str {
        &self.status_topic
    }

    pub(crate) fn new_sender(&self) -> MqttSender {
        MqttSender {
            sender: self.sender.clone(),
            connected: self.connected.subscribe(),
        }
    }

    pub(crate) async fn run_loop(mut self) -> anyhow::Result<()> {
        loop {
            match self.event_loop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(conn_ack))) => {
                    if conn_ack.code == ConnectReturnCode::Success {
                        debug!("Connected to MQTT broker");
                        // Ignoring the error is fine, as it'll only error if all receivers
                        // are dropped. If there are no receivers, there might be some in
                        // the future
                        #[allow(unused_must_use)]
                        {
                            self.connected.send(true);
                        }
                        // Set the online status immediately after we connect.
                        let mut sender = self.new_sender();
                        sender
                            .enqueue_publish(
                                self.status_topic.clone(),
                                QoS::AtLeastOnce,
                                &Status::Online,
                                true,
                            )
                            .await?;
                    } else {
                        error!(response_code = ?conn_ack.code, "Connection to MQTT broker refused.");
                        return Err(anyhow!("Connection to MQTT broker refused"));
                    }
                }
                Ok(event) => {
                    trace!(?event, "MQTT event processed")
                }
                // Attempt to handle the variety of error cases from the event loop.
                Err(rumqttc::ConnectionError::Network(net_err)) => {
                    // These are actually TLS errors internal to rumqttc.
                    // If the rumqttc::tls module (or the Error enum within it) are ever made
                    // accessible, this should get a bit simpler to handle instead of the
                    // dynamic downcasting mess we have here.
                    error!(error = ?net_err, "Encountered a network connection error");
                    if let Some(net_err_src) = net_err.source() {
                        // IO errors can be recoverable, treat everything else as unrecoverable
                        if net_err_src.is::<std::io::Error>() {
                            warn!(error = ?net_err, "MQTT client I/O error, retrying connection");
                            self.connected.send(false).unwrap();
                        } else {
                            return Err(net_err).context("MQTT network error");
                        }
                    } else {
                        return Err(net_err).context("MQTT network error of unknown kind");
                    }
                }
                Err(err @ rumqttc::ConnectionError::MqttState(rumqttc::StateError::Connect(_))) => {
                    // Any connection error is unrecoverable, as that (usually) means the
                    // config is incorrect.
                    error!(error = ?err, "Error connecting to MQTT broker");
                    return Err(err).context("Error connecting to MQTT broker");
                }
                Err(err @ rumqttc::ConnectionError::Io(_)) => {
                    error!(error = ?err, "Connection I/O error");
                    if let rumqttc::ConnectionError::Io(ref io_err) = err {
                        match io_err.kind() {
                            ErrorKind::ConnectionReset
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::BrokenPipe
                            | ErrorKind::TimedOut
                            | ErrorKind::Interrupted => {
                                self.connected.send(false).unwrap();
                                warn!(error = ?err, "Ignoring MQTT I/O error");
                            }
                            _ => {
                                return Err(err).context("MQTT I/O error");
                            }
                        }
                    } else {
                        return Err(err).context("Somehow an I/O error wasn't an I/O error");
                    }
                }
                Err(err) => {
                    // Treat all other errors as non-recoverable
                    error!(error = ?err, "Unknown client error");
                    return Err(err).context("Encountered MQTT client error");
                }
            }
        }
    }
}
