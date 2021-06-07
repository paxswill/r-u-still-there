// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::TryFrom;

use anyhow::{Context, Result};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use tracing::{debug};

#[derive(Debug)]
pub struct MqttCodec {
    decode_buffer: BytesMut,
    // TODO: This is a hac as mqttrs doesn't expose how many bytes a packet will encode to, and
    // looking at the source, it will panic when attempting to write to a too-small buffer.
    encode_buffer: BytesMut,
}

/// Initial amount of space to reserve for a new packet to be encoded, let's use 1 MB.
// TODO: More mqttrs hackage
const INITIAL_PACKET_BUFFER_SIZE: usize = 1024 * 1024;

#[derive(Debug)]
pub struct PacketBytes(Bytes);

impl MqttCodec {
    pub fn new() -> Self {
        Self {
            // Start off with a default capacity, but it will be enlarged later.
            decode_buffer: BytesMut::new(),
            encode_buffer: BytesMut::with_capacity(INITIAL_PACKET_BUFFER_SIZE),
        }
    }
}

impl Decoder for MqttCodec {
    type Item = PacketBytes;
    type Error = anyhow::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>> {
        /*
         * <rant>
         * Pre v0.4, mqttrs had a nice API, that was easy to use with tokio codecs. Then it 
         * changed to a slice based API, while not updating *any* of the documentation, and 
         * (as far as I can tell) not providing a way to see how many bytes a decoded packet 
         * consumed from a buffer.
         * </rant>
         */
        // Attempt to clone a packet from the given bytes, which seems to be the only way to know
        // how large a packet is available.
        // Resize the scratch buffer to be at least as large as the current receive buffer.
        self.decode_buffer.reserve(src.capacity());
        // The only possible error (as of mqttrs 0.4.1) is InvalidHeader, which isn't really
        // recoverable without resetting the connection.
        let cloned_length = mqttrs::clone_packet(src, &mut self.decode_buffer)
            .context("Invalid data received from MQTT broker")?;
        // 0 is being used (as of mqttrs 0.4.1) as a signal that there isn't a full packet.
        if cloned_length > 0 {
            debug!("Cloned {} bytes for a packet", cloned_length);
            let packet_data = self.decode_buffer.split().freeze();
            src.advance(cloned_length);
            return Ok(Some(PacketBytes(packet_data)));
        }
        Ok(None)
    }
}

impl<'a> Encoder<mqttrs::Packet<'a>> for MqttCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, packet: mqttrs::Packet<'a>, dst: &mut BytesMut) -> Result<()> {
        // TODO: eventually, I should contribute to mqttrs so that it can return how much is
        // actually needed.
        // TODO: The rest of the mqttrs hackage
        let packet_size = mqttrs::encode_slice(&packet, &mut self.encode_buffer)?;
        debug!(packet = ?packet, "Encoded an MQTT packet to {} bytes", packet_size);
        dst.reserve(packet_size);
        dst.put_slice(&self.encode_buffer[..packet_size]);
        // Always clear the temporary buffer
        self.encode_buffer.clear();
        todo!()
    }
}

impl<'a> TryFrom<&'a PacketBytes> for mqttrs::Packet<'a> {
    type Error = mqttrs::Error;

    fn try_from(packet: &'a PacketBytes) -> std::result::Result<Self, Self::Error> {
        mqttrs::decode_slice(&packet.0).map(|packet| {
            // Cloning the packet data ensures that a full packet is copied. None is only returned
            // when there isn't a full packet.
            packet.expect("there to be an entire packet after being cloned")
        })
    }
}
