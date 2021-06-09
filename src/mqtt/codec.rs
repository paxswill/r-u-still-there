// SPDX-License-Identifier: GPL-3.0-or-later
use std::error::Error as StdError;
use std::fmt;
use std::marker::PhantomData;

use bytes::BytesMut;
use mqttbytes::{v4, v5, Error as MqttError};
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug)]
pub struct MqttCodec<T>(PhantomData<T>);

/// The maximum packet size we'll acccept. This is to guard against a buggy (or malicious) broker.
const MAX_PACKET_SIZE: usize = 1024 * 1024 * 10;

/// A wrapper around [mqttbytes::Error] to add the required trait implementations for
/// Encode/Decode.
#[derive(Debug)]
pub enum Error {
    Mqtt(MqttError),
    Io(std::io::Error),
}

impl<T> MqttCodec<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl Decoder for MqttCodec<v4::Packet> {
    type Item = v4::Packet;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match v4::read(src, MAX_PACKET_SIZE) {
            Ok(packet) => Ok(Some(packet)),
            // Still need more bytes, but not an error
            Err(MqttError::InsufficientBytes(_)) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

impl Decoder for MqttCodec<v5::Packet> {
    type Item = v5::Packet;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match v5::read(src, MAX_PACKET_SIZE) {
            Ok(packet) => Ok(Some(packet)),
            // Still need more bytes, but not an error
            Err(MqttError::InsufficientBytes(_)) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

impl Encoder<v4::Packet> for MqttCodec<v4::Packet> {
    type Error = Error;

    fn encode(&mut self, packet: v4::Packet, dst: &mut BytesMut) -> Result<(), Self::Error> {
        match packet {
            v4::Packet::Connect(p) => p.write(dst),
            v4::Packet::ConnAck(p) => p.write(dst),
            v4::Packet::Publish(p) => p.write(dst),
            v4::Packet::PubAck(p) => p.write(dst),
            v4::Packet::PubRec(p) => p.write(dst),
            v4::Packet::PubRel(p) => p.write(dst),
            v4::Packet::PubComp(p) => p.write(dst),
            v4::Packet::Subscribe(p) => p.write(dst),
            v4::Packet::SubAck(p) => p.write(dst),
            v4::Packet::Unsubscribe(p) => p.write(dst),
            v4::Packet::UnsubAck(p) => p.write(dst),
            v4::Packet::PingReq => v4::PingReq.write(dst),
            v4::Packet::PingResp => v4::PingResp.write(dst),
            v4::Packet::Disconnect => v4::Disconnect.write(dst),
        }?;
        Ok(())
    }
}

impl Encoder<v5::Packet> for MqttCodec<v5::Packet> {
    type Error = Error;

    fn encode(&mut self, packet: v5::Packet, dst: &mut BytesMut) -> Result<(), Self::Error> {
        match packet {
            v5::Packet::Connect(p) => p.write(dst),
            v5::Packet::ConnAck(p) => p.write(dst),
            v5::Packet::Publish(p) => p.write(dst),
            v5::Packet::PubAck(p) => p.write(dst),
            v5::Packet::PubRec(p) => p.write(dst),
            v5::Packet::PubRel(p) => p.write(dst),
            v5::Packet::PubComp(p) => p.write(dst),
            v5::Packet::Subscribe(p) => p.write(dst),
            v5::Packet::SubAck(p) => p.write(dst),
            v5::Packet::Unsubscribe(p) => p.write(dst),
            v5::Packet::UnsubAck(p) => p.write(dst),
            v5::Packet::PingReq => v5::PingReq.write(dst),
            v5::Packet::PingResp => v5::PingResp.write(dst),
            v5::Packet::Disconnect(p) => p.write(dst),
        }?;
        Ok(())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mqtt(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
        }
    }
}

impl StdError for Error {}

impl From<MqttError> for Error {
    fn from(err: MqttError) -> Self {
        Self::Mqtt(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
