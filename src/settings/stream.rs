// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

use std::net;

fn default_address() -> net::IpAddr {
    net::IpAddr::from([0u8, 0u8, 0u8, 0u8])
}

fn default_port() -> u16 {
    9000u16
}

fn default_stream_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct StreamSettings {
    /// The address to bind the server to. Defaults to `127.0.0.1`.
    #[serde(default = "default_address")]
    address: net::IpAddr,

    /// The port to bind the server to. Default to `9000`.
    #[serde(default = "default_port")]
    port: u16,

    #[serde(default = "default_stream_enabled")]
    pub mjpeg: bool,
}

impl From<StreamSettings> for net::SocketAddr {
    fn from(settings: StreamSettings) -> Self {
        match settings.address {
            net::IpAddr::V4(ip) => net::SocketAddr::from((ip, settings.port)),
            net::IpAddr::V6(ip) => net::SocketAddr::from((ip, settings.port)),
        }
    }
}

impl<'a> Default for StreamSettings {
    fn default() -> Self {
        Self {
            address: default_address(),
            port: default_port(),
            mjpeg: default_stream_enabled(),
        }
    }
}

#[cfg(test)]
mod stream_test {
    use super::{default_address, default_port, default_stream_enabled, StreamSettings};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn default_settings() {
        let parsed: Result<StreamSettings, _> = toml::from_str("");
        assert!(parsed.is_ok(), "Failed to parse empty TOML");
        let parsed = parsed.unwrap();
        let expected = StreamSettings::default();
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
            mjpeg: default_stream_enabled(),
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
            mjpeg: default_stream_enabled(),
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
            mjpeg: default_stream_enabled(),
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
            mjpeg: default_stream_enabled(),
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
            mjpeg: default_stream_enabled(),
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
            mjpeg: default_stream_enabled(),
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
            mjpeg: default_stream_enabled(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn string_port() {
        let parsed: Result<StreamSettings, _> = toml::from_str("port = \"foo\"");
        assert!(parsed.is_err(), "Incorrectly parsed string as port number");
    }

    #[test]
    fn mjpeg_on() {
        let parsed: Result<StreamSettings, _> = toml::from_str("mjpeg = true");
        assert!(parsed.is_ok(), "Failed to parse MJPEG enabled");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: default_address(),
            port: default_port(),
            mjpeg: true,
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn mjpeg_off() {
        let parsed: Result<StreamSettings, _> = toml::from_str("mjpeg = false");
        assert!(parsed.is_ok(), "Failed to parse MJPEG disabled");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            address: default_address(),
            port: default_port(),
            mjpeg: false,
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn mjpeg_invalid() {
        let parsed: Result<StreamSettings, _> = toml::from_str("mjpeg = \"foo\"");
        assert!(
            parsed.is_err(),
            "Incorrectly parsed bad MJPEG configuration"
        );
    }
}