// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::iter::Iterator;
use std::mem::discriminant;

use mac_address::MacAddress;
use serde::de::{Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::util::is_default;

/// The types of connections that can be associated with a device in the Home Assistant device
/// registry.
///
/// There isn't a great reference for what kinds of connections are supported. The docs give MAC
/// addresses as an example, but by peeking in the source we can see that UPnP and Zigbee IDs are
/// also supported.
#[derive(Clone, Debug, PartialEq)]
pub enum Connection {
    MacAddress(MacAddress),
    // TODO: see if this actually matches the spec. This will also be a bit difficult as I'm not
    // sure of any integrations that actually use this :/
    UPnP(Uuid),
    Zigbee(String),
}
// MacAddress doesn't implement Eq or Hash, so we get to implement (or mark) those ourselves.
impl std::cmp::Eq for Connection {}
#[allow(clippy::clippy::derive_hash_xor_eq)]
impl Hash for Connection {
    fn hash<H: Hasher>(&self, state: &mut H) {
        discriminant(self).hash(state);
        match self {
            Connection::MacAddress(mac) => {
                mac.bytes().hash(state);
            }
            Connection::UPnP(upnp) => {
                upnp.hash(state);
            }
            Connection::Zigbee(addr) => {
                addr.hash(state);
            }
        }
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ConnectionTag {
    Mac,
    UPnP,
    Zigbee,
}

impl Serialize for Connection {
    /// Serialize a Connection as a 2-tuple
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tuple_serializer = serializer.serialize_tuple(2)?;
        match self {
            Connection::MacAddress(mac) => {
                tuple_serializer.serialize_element(&ConnectionTag::Mac)?;
                tuple_serializer.serialize_element(mac)?;
            }
            Connection::UPnP(uuid) => {
                tuple_serializer.serialize_element(&ConnectionTag::UPnP)?;
                tuple_serializer.serialize_element(uuid)?;
            }
            Connection::Zigbee(addr) => {
                tuple_serializer.serialize_element(&ConnectionTag::Zigbee)?;
                tuple_serializer.serialize_element(addr)?;
            }
        };
        tuple_serializer.end()
    }
}

impl<'de> Deserialize<'de> for Connection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ConnectionVisitor;

        impl<'de> Visitor<'de> for ConnectionVisitor {
            type Value = Connection;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a connection tuple")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                // It's almost missing_field(), but there are multiple fields it could be
                let missing_tag =
                    V::Error::custom("Missing one of the connection type names as a key");
                let tag = map.next_key()?.ok_or(missing_tag)?;
                let connection_type = match tag {
                    ConnectionTag::Mac => {
                        let mac = map.next_value()?;
                        Connection::MacAddress(mac)
                    }
                    ConnectionTag::UPnP => {
                        let uuid = map.next_value()?;
                        Connection::UPnP(uuid)
                    }
                    ConnectionTag::Zigbee => {
                        let addr = map.next_value()?;
                        Connection::Zigbee(addr)
                    }
                };
                Ok(connection_type)
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                // There should be exactly two elements in the sequence
                let expect_two = || {
                    V::Error::invalid_length(
                        2,
                        &"there should be exactly two elements in a component array",
                    )
                };

                let tag = seq.next_element()?.ok_or_else(expect_two)?;
                let connection_type = match tag {
                    ConnectionTag::Mac => {
                        let mac = seq.next_element()?.ok_or_else(expect_two)?;
                        Connection::MacAddress(mac)
                    }
                    ConnectionTag::UPnP => {
                        let uuid = seq.next_element()?.ok_or_else(expect_two)?;
                        Connection::UPnP(uuid)
                    }
                    ConnectionTag::Zigbee => {
                        let addr = seq.next_element()?.ok_or_else(expect_two)?;
                        Connection::Zigbee(addr)
                    }
                };
                Ok(connection_type)
            }
        }

        deserializer.deserialize_any(ConnectionVisitor)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Device {
    #[serde(alias = "cns", default, skip_serializing_if = "is_default")]
    connections: HashSet<Connection>,

    #[serde(alias = "ids", default, skip_serializing_if = "is_default")]
    identifiers: HashSet<String>,

    #[serde(alias = "mf", default, skip_serializing_if = "is_default")]
    pub manufacturer: Option<String>,

    #[serde(alias = "mdl", default, skip_serializing_if = "is_default")]
    pub model: Option<String>,

    // No alias for 'name'
    #[serde(default, skip_serializing_if = "is_default")]
    pub name: Option<String>,

    #[serde(alias = "sa", default, skip_serializing_if = "is_default")]
    pub suggested_area: Option<String>,

    #[serde(alias = "sw", default, skip_serializing_if = "is_default")]
    pub sw_version: Option<String>,

    // No alias for 'via_device' either
    #[serde(default, skip_serializing_if = "is_default")]
    pub via_device: Option<String>,
}

impl Device {
    /// Add a MAC address to this device.
    pub fn add_mac_connection(&mut self, mac: MacAddress) {
        self.connections.insert(Connection::MacAddress(mac));
    }

    // Skipping the UPnP and ZigBee access methods as I'm not planning on using them.

    /// Iteratoe over the MAC addresses currently associated with this device.
    pub fn mac_addresses(&self) -> impl Iterator<Item = &MacAddress> {
        self.connections.iter().filter_map(|con| match con {
            Connection::MacAddress(mac) => Some(mac),
            _ => None,
        })
    }

    pub fn add_identifier<S>(&mut self, id: S)
    where
        S: Into<String>,
    {
        self.identifiers.insert(id.into());
    }

    pub fn identifiers(&self) -> impl Iterator<Item = &String> {
        self.identifiers.iter()
    }
}
