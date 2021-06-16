// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::anyhow;
use hmac::{Hmac, Mac, NewMac};
use machine_uid::machine_id::get_machine_id;
use rumqttc::{ClientConfig, Transport};
use serde::Deserialize;
use sha2::Sha256;
use tracing::{debug, trace, warn};
use url::Url;

use std::convert::{TryFrom, TryInto};
use std::fmt;
use std::str::FromStr;

use super::external_value::ExternalValue;

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_MQTT_PORT: u16 = 1883;
pub const DEFAULT_MQTTS_PORT: u16 = 8883;
const APPLICATION_KEY: &[u8; 16] =
    b"\x64\x6c\x30\xc3\x41\xd7\x47\x40\x8b\x1e\xe0\x78\xf7\x4c\x73\xe0";

#[derive(Deserialize, PartialEq)]
pub struct MqttSettings {
    /// A name for the base topic for this device.
    pub(super) name: String,

    /// Provide a persistent unique identifier for this device.
    ///
    /// This value need to be unique across different devices, but also persistent over the life of
    /// the device. By default the systemd `machine-id` is used as a seed to generate an ID
    /// automatically, but there are some uses for manually specifying it (ex: migrating an
    /// existing setup to a new installation, or using a volatile system that regenerates its
    /// `machine-id` on every boot).
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

    /// Enable MQTT keep-alive.
    ///
    /// Periodically the client will ping the server so the server knows the connection is still
    /// active. Specified in seconds. 0 is the same as disabled.
    #[serde(default)]
    keep_alive: Option<u16>,

    /// Enable Home Assistant integration.
    ///
    /// When enabled, entities will be automatically added to Home Assistant using MQTT discovery.
    /// Do note that the MJPEG stream is *not* able to be automatically added in this way, you will
    /// need to add it manually.
    #[serde(default = "MqttSettings::default_home_assistant")]
    pub(super) home_assistant: bool,

    /// The topic prefix used for Home Assistant MQTT discovery.
    ///
    /// Defaults to "homeassistant"
    #[serde(default = "MqttSettings::default_home_assistant_topic")]
    pub(super) home_assistant_topic: String,

    /// Retain Home Assistant MQTT discovery configuration on the MQTT broker.
    ///
    /// **In almost all cases this option should be enabled, and the default is to be enabled.**
    ///
    /// By disabling this, the entity configuration will not be stored on the MQTT broker, and Home
    /// Assistant will only receive it when r-u-still-there starts up.
    #[serde(default = "MqttSettings::default_home_assistant_retain")]
    pub(super) home_assistant_retain: bool,
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

    /// The default Home Assistant MQTT discovery topic prefix.
    fn default_home_assistant_topic() -> String {
        "homeassistant".into()
    }

    /// The default value for whether or not to retain Home Assistant MQTT discovery configuration.
    fn default_home_assistant_retain() -> bool {
        true
    }

    /// Access the server URL.
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
                // The full output of the HMAC is 256 bits, but we only need 128 (aka 16 bytes).
                let uid_bytes = &mac.finalize().into_bytes()[..16];
                // Encode the output as base64, the full output as hex is a bit long
                let uid = base64::encode_config(uid_bytes, base64::URL_SAFE_NO_PAD);
                debug!(unique_id = %uid, "generated unique ID");
                uid
            }
        }
    }
}

impl fmt::Debug for MqttSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttSettings")
            .field("name", &self.name)
            .field("unique_id", &self.unique_id)
            .field("username", &self.username)
            // <Cthon98> lol, yes. See, when YOU type hunter2, it shows to us as *******
            // Censor the password (if any) to keep it out of the logs.
            .field("password", &self.password.as_ref().map(|_| "*******"))
            .field("server", &self.server)
            .field("keep_alive", &self.keep_alive)
            .field("home_assistant", &self.home_assistant)
            .field("home_assistant_topic", &self.home_assistant_topic)
            .field("home_assistant_retain", &self.home_assistant_retain)
            .finish()
    }
}

impl TryFrom<&MqttSettings> for rumqttc::MqttOptions {
    type Error = anyhow::Error;

    fn try_from(user_config: &MqttSettings) -> anyhow::Result<Self> {
        let url = user_config.server_url();
        let host_str = url
            .host_str()
            .ok_or(anyhow!("MQTT URL somehow doesn't have a host"))?;
        let port = url.port().ok_or(anyhow!("Unset port for the MQTT URL"))?;
        let mut options = Self::new(user_config.name.clone(), host_str, port);
        match url.scheme() {
            "mqtts" => {
                let mut tls_config = ClientConfig::new();
                // If disabling client verification was ever supported, it would be done here.
                // On second thought, provide a way to use a custom certificate as the trust root,
                // but not completely disable verification.
                tls_config
                    .root_store
                    .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
                options.set_transport(Transport::tls_with_config(tls_config.into()));
            }
            "mqtt" => {
                options.set_transport(Transport::tcp());
            }
            _ => return Err(anyhow!("unknown MQTT scheme")),
        }
        // MQTT3/4 authentication
        if let Some(username) = &user_config.username {
            let password = user_config
                .password
                .as_ref()
                .map_or("".to_string(), |p| p.0.clone());
            debug!("Adding credentials to MQTT client configuration");
            options.set_credentials(username, &password);
        }
        // Explicit keep alive setting
        if let Some(keep_alive) = user_config.keep_alive {
            options.set_keep_alive(keep_alive);
        }
        Ok(options)
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
            keep_alive: None,
            home_assistant: false,
            home_assistant_topic: "homeassistant".into(),
            home_assistant_retain: true,
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
