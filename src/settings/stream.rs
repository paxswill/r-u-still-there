// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

use std::collections::HashMap;
use std::net;

use crate::stream::{mjpeg::MjpegStream, VideoStream};

fn default_address() -> net::IpAddr {
    net::IpAddr::from([127u8, 0u8, 0u8, 1u8])
}

fn default_port() -> u16 {
    9000u16
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct StreamSettings {
    /// The address to bind the server to. Defaults to `127.0.0.1`.
    #[serde(default = "default_address")]
    address: net::IpAddr,

    /// The port to bind the server to. Default to `9000`.
    #[serde(default = "default_port")]
    port: u16,
}

impl From<StreamSettings> for net::SocketAddr {
    fn from(settings: StreamSettings) -> net::SocketAddr {
        match settings.address {
            net::IpAddr::V4(ip) => net::SocketAddr::from((ip, settings.port)),
            net::IpAddr::V6(ip) => net::SocketAddr::from((ip, settings.port)),
        }
    }
}

#[cfg(test)]
mod stream_test {
    use super::{default_address, default_port, StreamSettings};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn default_settings() {
        let parsed: Result<StreamSettings, _> = toml::from_str("");
        assert!(parsed.is_ok(), "Failed to parse empty TOML");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: default_address(),
            port: default_port(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn ipv4_local_address() {
        let parsed: Result<StreamSettings, _> = toml::from_str("address = \"127.0.0.1\"");
        assert!(parsed.is_ok(), "Failed to parse IPv4 address");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: IpAddr::from(Ipv4Addr::new(127, 0, 0, 1)),
            port: default_port(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn ipv4_wildcard_address() {
        let parsed: Result<StreamSettings, _> = toml::from_str("address = \"0.0.0.0\"");
        assert!(parsed.is_ok(), "Failed to parse IPv4 address");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: IpAddr::from(Ipv4Addr::new(0, 0, 0, 0)),
            port: default_port(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn ipv4_normal_address() {
        // Using an IP address from TEST-NET-1 (see RFC 5737)
        let parsed: Result<StreamSettings, _> = toml::from_str("address = \"192.0.2.20\"");
        assert!(parsed.is_ok(), "Failed to parse IPv4 address");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: IpAddr::from(Ipv4Addr::new(192, 0, 2, 20)),
            port: default_port(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn ipv6_local_address() {
        let parsed: Result<StreamSettings, _> = toml::from_str("address = \"::1\"");
        assert!(parsed.is_ok(), "Failed to parse IPv6 address");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: IpAddr::from(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)),
            port: default_port(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn ipv6_wildcard_address() {
        let parsed: Result<StreamSettings, _> = toml::from_str("address = \"::\"");
        assert!(parsed.is_ok(), "Failed to parse IPv6 address");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: IpAddr::from(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)),
            port: default_port(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn ipv6_normal_address() {
        // Using a documentation IP address (see RFC 3849)
        let parsed: Result<StreamSettings, _> =
            toml::from_str("address = \"2001:db8:dead:beef::1\"");
        assert!(parsed.is_ok(), "Failed to parse IPv6 address");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: IpAddr::from(Ipv6Addr::new(0x2001, 0xdb8, 0xdead, 0xbeef, 0, 0, 0, 1)),
            port: default_port(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn port() {
        let parsed: Result<StreamSettings, _> = toml::from_str("port = 1337");
        assert!(parsed.is_ok(), "Failed to parse port number");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: default_address(),
            port: 1337u16,
        };
        assert_eq!(parsed, expected);
    }
}
