// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::anyhow;
use hmac::{Hmac, Mac, NewMac};
use machine_uid::machine_id::get_machine_id;
use mqttbytes::v4::Login;
use mqttbytes::{v4, v5, Protocol};
use serde::Deserialize;
use sha2::Sha256;
use tracing::{debug, trace, warn};
use url::Url;

use std::convert::{TryFrom, TryInto};
use std::str::FromStr;

use super::external_value::ExternalValue;

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_MQTT_PORT: u16 = 1883;
pub const DEFAULT_MQTTS_PORT: u16 = 8883;
const APPLICATION_KEY: &[u8; 16] =
    b"\x64\x6c\x30\xc3\x41\xd7\x47\x40\x8b\x1e\xe0\x78\xf7\x4c\x73\xe0";

#[derive(Deserialize, Debug, PartialEq)]
pub struct MqttSettings {
    /// A name for the base topic for this device.
    name: String,

    /// Override the unique ID for this device.
    ///
    /// The unique ID is only used with the Home Assistant integration. If not provided, an ID is
    /// generated automatically. The generated ID should be stable across a system install, but if
    /// you want to guarantee that (or re-use an existing ID) you can specify it here. If
    /// specified, the ID is used *exactly* as written.
    #[serde(default)]
    unique_id: Option<String>,

    /// The MQTT server username, if required.
    #[serde(default)]
    username: Option<String>,

    /// The MQTT server password, if required.
    ///
    /// While a password *can* be specified directly in a configuration file, it is recommended to
    /// provide it either in an environment variable, or in a separate file with the minimal file
    /// permissions necessary. This configuration value can be given either as a plain string, or
    /// as a map/object of a key "file" to a string. In the first case, the string value is treated
    /// as the password. In the second, the inner value is a path to a file, the contents of which
    /// are read in and used as the password.
    password: Option<ExternalValue>,

    /// A URL for the MQTT server to connect to. If not given, the scheme 'mqtt' is assumed. Valid
    /// schemes are 'mqtt' for MQTT over TCP and 'mqtts' for MQTT over TLS. If a port is not given,
    /// 1883 is used for MQTT over TCP, and 8883 for MQTT over TLS.
    server: MqttUrl,

    /// Enable Home Assistant integration.
    ///
    /// When enabled, entities will be automatically added to Home Assistant using MQTT discovery.
    /// Do note that the MJPEG stream is *not* able to be automatically added in this way, you will
    /// need to add it manually.
    #[serde(default = "MqttSettings::default_home_assistant")]
    home_assistant: bool,

    /// Enable MQTT keep-alive.
    ///
    /// Periodically the client will ping the server so the server knows the connection is still
    /// active. Specified in seconds. 0 is the same as disabled.
    #[serde(default)]
    keep_alive: Option<u16>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(try_from = "Url")]
pub struct MqttUrl(Url);

impl TryFrom<Url> for MqttUrl {
    type Error = anyhow::Error;

    /// Attempt to create an [MqttUrl] from a [Url].
    ///
    /// It is an arror if the URL scheme is something other than 'mqtt' or 'mqtts'. The default
    /// ports for those schemes are also applied if no port is given.
    fn try_from(mut url: Url) -> anyhow::Result<Self> {
        match url.scheme() {
            "mqtt" | "mqtts" => (),
            invalid => return Err(anyhow!("invalid scheme '{}'", invalid)),
        }
        if url.port().is_none() {
            match url.scheme() {
                "mqtt" => url
                    .set_port(Some(DEFAULT_MQTT_PORT))
                    .map_err(|_| anyhow!("unable set default MQTT over TCP port"))?,
                "mqtts" => url
                    .set_port(Some(DEFAULT_MQTTS_PORT))
                    .map_err(|_| anyhow!("unable to set default MQTT over TLS port"))?,
                _ => unreachable!(),
            }
        }
        Ok(Self(url))
    }
}

impl<'a> From<&'a MqttUrl> for (&'a str, u16) {
    fn from(url: &'a MqttUrl) -> Self {
        (
            url.0
                .host_str()
                .expect("the server to have a host specified"),
            url.0
                .port()
                .expect("the server validation to have set an explicit port"),
        )
    }
}

impl FromStr for MqttUrl {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let url: url::Url = s.parse()?;
        url.try_into()
    }
}

impl MqttSettings {
    /// The default value for the home_assistant field.
    fn default_home_assistant() -> bool {
        false
    }

    pub fn server_url(&self) -> &Url {
        &self.server.0
    }

    /// Get the unique ID for this device.
    ///
    /// If one was provided, use that. If not, retrieve a machine-specific ID from the OS and hash
    /// it. If a machine-specific ID is not able to be found, the configured name is used instead
    /// (also hashed).
    pub fn unique_id(&self) -> String {
        match &self.unique_id {
            Some(uid) => uid.clone(),
            None => {
                let machine_id: Vec<u8> = match get_machine_id() {
                    Ok(machine_id) => {
                        let hex_digits: String = machine_id
                            .to_ascii_lowercase()
                            .matches(|c: char| c.is_ascii_hexdigit())
                            .collect();
                        // trace level can log possibly sensitive information, which includes the
                        // raw (unhashed) machine ID
                        trace!(machine_id = %hex_digits, "extracted machine ID");
                        hex::decode(hex_digits).unwrap()
                    }
                    Err(e) => {
                        warn!(error = ?e, "Unable to get machine ID, using '{}' instead", self.name);
                        self.name.as_bytes().into()
                    }
                };
                // Create an HMAC of the machine ID, keyed with APPLICATION_KEY. This is a privacy
                // preservation measure as recommended by the systemd machine-id documentation. It
                // seems like good practice overall so I'm applying it to the other platforms as
                // well. The systemd function for this only returns a 16 byte value, but I'm not
                // sure how they get down to that from the 32 bytes from the HMAC, so I'll just
                // leave it all in there.
                let mut mac = HmacSha256::new_from_slice(APPLICATION_KEY)
                    .expect("HMAC can be created from embedded key");
                mac.update(&machine_id);
                let uid = hex::encode(mac.finalize().into_bytes());
                debug!(unique_id = %uid, "generated unique ID");
                uid
            }
        }
    }
}

impl From<&MqttSettings> for v4::Connect {
    fn from(settings: &MqttSettings) -> Self {
        let login: Option<v4::Login> = if let Some(username) = &settings.username {
            let password: String = settings
                .password
                .as_ref()
                .map_or("".to_string(), |p| p.0.clone());
            Some(Login::new(username.clone(), password))
        } else {
            None
        };
        Self {
            // Only MQTT 3.1.1 for now, but 5 should be implemented by mqttrs at some point.
            protocol: Protocol::V4,
            keep_alive: settings.keep_alive.unwrap_or(0),
            // Using the name from the settings for now, but might change this later.
            client_id: settings.name.clone(),
            // Always start with a clean session
            clean_session: true,
            last_will: None,
            login,
        }
    }
}

#[cfg(test)]
mod test {
    use super::MqttSettings;

    #[test]
    fn defaults() {
        let source = r#"
        name = "example"
        server = "mqtt://127.0.0.1"
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: MqttSettings = parsed.unwrap();
        let expected = MqttSettings {
            name: "example".to_string(),
            unique_id: None,
            username: None,
            password: None,
            server: "mqtt://127.0.0.1".parse().unwrap(),
            home_assistant: false,
            keep_alive: None,
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn specified_unique_id() {
        let unique_id = "abcdefghijklmnopqrstuvwxyz0123456789";
        let source = format!(
            r#"
        name = "example"
        server = "mqtt://127.0.0.1"
        unique_id = "{}"
        "#,
            unique_id
        );
        let parsed = toml::from_str(&source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: MqttSettings = parsed.unwrap();
        assert_eq!(parsed.unique_id(), unique_id.to_string());
    }

    #[test]
    fn generate_unique_id() {
        let source = r#"
        name = "example"
        server = "mqtt://127.0.0.1"
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: MqttSettings = parsed.unwrap();
        let unique_id = parsed.unique_id();
        assert!(
            unique_id.len() == 64,
            "Unique ID ({}) is not 64 hex digits long (is /etc/machine-id not available?)",
            unique_id
        );
    }
}

#[cfg(test)]
mod mqtt_url_test {
    use super::{MqttUrl, DEFAULT_MQTTS_PORT, DEFAULT_MQTT_PORT};
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct UrlWrapper {
        field: MqttUrl,
    }

    #[test]
    fn mqtt_url_allowed_schemes() {
        let parsed_mqtt: UrlWrapper = toml::from_str(
            r#"
        field = "mqtt://example.com"
        "#,
        )
        .expect("to be able to parse an mqtt URL");
        let parsed_mqtts: UrlWrapper = toml::from_str(
            r#"
        field = "mqtts://example.com"
        "#,
        )
        .expect("to be able to parse an mqtts URL");
        let mqtt_url = parsed_mqtt.field.0;
        let mqtts_url = parsed_mqtts.field.0;
        assert_eq!(mqtt_url.scheme(), "mqtt");
        assert_eq!(mqtts_url.scheme(), "mqtts");
    }

    #[test]
    fn mqtt_url_unknown_scheme() {
        // Using websocket scheme specifically because this program does not support them, but they
        // are a standard MQTT transport.
        let parse_result: Result<UrlWrapper, _> = toml::from_str(
            r#"
        field = "ws://example.com"
        "#,
        );
        assert!(
            parse_result.is_err(),
            "WebSocket scheme was accepted: {:?}",
            parse_result
        );
    }

    #[test]
    fn mqtt_url_default_mqtt_port() {
        let parse_result = toml::from_str(
            r#"
        field = "mqtt://example.com"
        "#,
        );
        assert!(
            parse_result.is_ok(),
            "Unable to parse mqtt URL: {:?}",
            parse_result
        );
        let wrapper: UrlWrapper = parse_result.unwrap();
        let url = wrapper.field.0;
        assert_eq!(
            url.port(),
            Some(DEFAULT_MQTT_PORT),
            "Incorrect default MQTT port"
        );
    }

    #[test]
    fn mqtt_url_default_mqtts_port() {
        let parse_result = toml::from_str(
            r#"
        field = "mqtts://example.com"
        "#,
        );
        assert!(
            parse_result.is_ok(),
            "Unable to parse mqtts URL: {:?}",
            parse_result
        );
        let wrapper: UrlWrapper = parse_result.unwrap();
        let url = wrapper.field.0;
        assert_eq!(
            url.port(),
            Some(DEFAULT_MQTTS_PORT),
            "Incorrect default MQTTS port"
        );
    }

    #[test]
    fn mqtt_url_custom_port() {
        let parse_result = toml::from_str(
            r#"
        field = "mqtts://example.com:1337"
        "#,
        );
        assert!(
            parse_result.is_ok(),
            "Unable to parse mqtts URL: {:?}",
            parse_result
        );
        let wrapper: UrlWrapper = parse_result.unwrap();
        let url = wrapper.field.0;
        assert_eq!(
            url.port(),
            Some(1337),
            "No the expected explicit port number"
        );
    }

    #[test]
    fn mqtt_url_into_socket_socket_pair() {
        let url: MqttUrl = "mqtt://example.test"
            .parse()
            .expect("to be able to parse a string to an MqttUrl");
        let socket_pair: (&str, u16) = (&url).into();
        assert_eq!(socket_pair.0, "example.test");
        assert_eq!(socket_pair.1, DEFAULT_MQTT_PORT);
    }
}
