// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

use std::net;

#[derive(Debug, Deserialize, PartialEq)]
pub(crate) struct StreamSettings {
    /// The address to bind the server to. Defaults to `127.0.0.1`.
    #[serde(default = "StreamSettings::default_address")]
    address: net::IpAddr,

    /// The port to bind the server to. Default to `9000`.
    #[serde(default = "StreamSettings::default_port")]
    port: u16,

    /// MJPEG-specific settings.
    #[serde(default)]
    pub(crate) mjpeg: MjpegSettings,
}

impl StreamSettings {
    /// Test if any streams are enabled.
    pub(crate) fn any_streams_enabled(&self) -> bool {
        // Right now there's only MJPEG streams, so this isn't a very useful check, but it'll be
        // useful later.
        self.mjpeg.enabled
    }

    fn default_address() -> net::IpAddr {
        net::IpAddr::from([127u8, 0u8, 0u8, 1u8])
    }

    fn default_port() -> u16 {
        9000u16
    }
}

impl From<StreamSettings> for net::SocketAddr {
    fn from(settings: StreamSettings) -> Self {
        match settings.address {
            net::IpAddr::V4(ip) => net::SocketAddr::from((ip, settings.port)),
            net::IpAddr::V6(ip) => net::SocketAddr::from((ip, settings.port)),
        }
    }
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            address: Self::default_address(),
            port: Self::default_port(),
            mjpeg: MjpegSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub(crate) struct MjpegSettings {
    /// Whether or not the MJPEG video stream should be enabled.
    #[serde(default = "MjpegSettings::default_enabled")]
    pub(crate) enabled: bool,
}

impl MjpegSettings {
    fn default_enabled() -> bool {
        true
    }
}

impl Default for MjpegSettings {
    fn default() -> Self {
        Self {
            enabled: Self::default_enabled(),
        }
    }
}

#[cfg(test)]
mod stream_test {
    use super::{MjpegSettings, StreamSettings};
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
            ..StreamSettings::default()
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
            ..StreamSettings::default()
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
            ..StreamSettings::default()
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
            ..StreamSettings::default()
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
            ..StreamSettings::default()
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
            ..StreamSettings::default()
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn port() {
        let parsed: Result<StreamSettings, _> = toml::from_str("port = 1337");
        assert!(parsed.is_ok(), "Failed to parse port number");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            port: 1337u16,
            ..StreamSettings::default()
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
        let parsed: Result<StreamSettings, _> = toml::from_str("mjpeg.enabled = true");
        assert!(parsed.is_ok(), "Failed to parse MJPEG enabled");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            mjpeg: MjpegSettings {
                enabled: true,
                ..MjpegSettings::default()
            },
            ..StreamSettings::default()
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn mjpeg_off() {
        let parsed: Result<StreamSettings, _> = toml::from_str("mjpeg.enabled = false");
        assert!(parsed.is_ok(), "Failed to parse MJPEG disabled");
        let parsed = parsed.unwrap();
        let expected = StreamSettings {
            mjpeg: MjpegSettings {
                enabled: false,
                ..MjpegSettings::default()
            },
            ..StreamSettings::default()
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn mjpeg_invalid() {
        let parsed: Result<StreamSettings, _> = toml::from_str("mjpeg.enabled = \"foo\"");
        assert!(
            parsed.is_err(),
            "Incorrectly parsed bad MJPEG configuration"
        );
    }
}
