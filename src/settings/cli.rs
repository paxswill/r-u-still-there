// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;
use structopt::clap::{AppSettings, ArgGroup};
use structopt::StructOpt;
use toml::value::{Table, Value};

use std::borrow::ToOwned;
use std::convert::TryInto;
use std::net;
use std::num::NonZeroU8;
use std::path::PathBuf;

use crate::camera::{Bus, CameraSettings};
use crate::mqtt::MqttUrl;
use crate::temperature::TemperatureUnit;
use crate::util::parse_int_decimal_hex;

use super::Settings;

#[derive(Clone, Debug, Default, StructOpt)]
#[structopt(setting(AppSettings::DeriveDisplayOrder))]
#[structopt(group = ArgGroup::with_name("mjpeg"))]
#[structopt(group = ArgGroup::with_name("home_assistant"))]
pub(crate) struct Args {
    /// Path to a configuration file.
    #[structopt(short, long, parse(from_os_str))]
    #[structopt(env = "RUSTILLTHERE_CONFIG")]
    pub(crate) config_path: Option<PathBuf>,

    /// The kind of camera being used.
    #[structopt(short = "C", long, possible_values(CameraSettings::KINDS))]
    #[structopt(env = "RUSTILLTHERE_CAMERA")]
    pub(crate) camera_kind: Option<String>,

    /// The I2C bus the camera is connected to.
    #[structopt(short = "b", long)]
    #[structopt(env = "RUSTILLTHERE_I2C_BUS")]
    pub(crate) i2c_bus: Option<Bus>,

    /// The I2C address the camera is available at.
    #[structopt(short = "a", long, parse(try_from_str = parse_int_decimal_hex))]
    #[structopt(env = "RUSTILLTHERE_I2C_ADDRESS")]
    pub(crate) i2c_address: Option<u8>,

    /// The camera frame rate to use.
    #[structopt(short, long)]
    #[structopt(env = "RUSTILLTHERE_FPS")]
    pub(crate) frame_rate: Option<NonZeroU8>,

    /// The size of each camera pixel in the rendered image.
    #[structopt(short, long)]
    #[structopt(env = "RUSTILLTHERE_GRID_SIZE")]
    pub(crate) grid_size: Option<usize>,

    /// The unit to display the temperature in.
    #[structopt(short = "u", long = "units")]
    #[structopt(env = "RUSTILLTHERE_UNITS")]
    pub(crate) temperature_units: Option<TemperatureUnit>,

    /// The color scheme to use when rendering the thermal image.
    #[structopt(short = "o", long)]
    #[structopt(env = "RUSTILLTHERE_COLORS")]
    pub(crate) colors: Option<super::gradient::Gradient>,

    /// The IP address the streaming server should listen on.
    #[structopt(short = "l", long = "listen-address")]
    #[structopt(env = "RUSTILLTHERE_LISTEN_ADDRESS")]
    pub(crate) listen_address: Option<net::IpAddr>,

    /// The port number to bind the streaming server to.
    #[structopt(short = "p", long = "listen-port")]
    #[structopt(env = "RUSTILLTHERE_LISTEN_PORT")]
    pub(crate) listen_port: Option<u16>,

    /// Enable MJPEG streaming.
    #[structopt(short = "m", long = "mjpeg", group = "mjpeg")]
    pub(super) enable_mjpeg: bool,

    /// Disable MJPEG streaming.
    #[structopt(short = "M", long = "no-mjpeg", group = "mjpeg")]
    pub(super) disable_mjpeg: bool,

    /// The name for this device as exposed on the MQTT server.
    ///
    /// This name is used as part of the topics the sensor values are published to.
    #[structopt(short = "N", long)]
    #[structopt(env = "RUSTILLTHERE_MQTT_NAME")]
    pub(crate) mqtt_name: Option<String>,

    /// The (optional) username to used to connect to the MQTT broker.
    ///
    /// An empty string is interpreted as no username.
    #[structopt(short = "U", long)]
    #[structopt(env = "RUSTILLTHERE_MQTT_USERNAME")]
    pub(crate) mqtt_username: Option<String>,

    /// The (optional) password used to connect to the MQTT broker.
    ///
    /// An empty string is interpreted as no password.
    #[structopt(short = "P", long)]
    #[structopt(env = "RUSTILLTHERE_MQTT_PASSWORD", hide_env_values = true)]
    pub(crate) mqtt_password: Option<String>,

    /// The URL to the MQTT broker.
    ///
    /// The only schemes accepted are 'mqtt' and 'mqtts', with default ports of 1883 and 8883
    /// respectively.
    #[structopt(short = "S", long, parse(try_from_str))]
    #[structopt(env = "RUSTILLTHERE_MQTT_SERVER")]
    pub(crate) mqtt_server: Option<MqttUrl>,

    /// Enable Home Assistant integration.
    #[structopt(long = "home-assistant", group = "home_assistant")]
    pub(super) enable_home_assistant: bool,

    /// Disable Home Assistant integration.
    #[structopt(long = "no-home-assistant", group = "home_assistant")]
    pub(super) disable_home_assistant: bool,

    #[cfg(feature = "mock_camera")]
    /// The file to use for mock camera data.
    ///
    /// When the mock camera is being used, this given path is used as the source of camera data.
    /// When other cameras are being used, their data is written to this file. When used as a
    /// destination, any existing data will be overwritten.
    #[structopt(env = "RUSTILLTHERE_MOCK_FILE", long = "mock-file", parse(from_os_str))]
    pub(crate) path: Option<PathBuf>,

    #[cfg(feature = "mock_camera")]
    /// How mock camera data should be repeated.
    ///
    /// This option is only relevant when the mock camera is being used. If not specified, "loop"
    /// is used.
    #[structopt(long = "repeat-mode", possible_values(crate::camera::RepeatMode::KINDS))]
    pub(crate) mock_repeat_mode: Option<crate::camera::RepeatMode>,
}

/// DRY macro for merging in optional CLI arguments.
///
/// It takes at least 4 arguments. The first is the configuration [table][toml::value::Table] for
/// the configuration file. The second is either the name of a [toml::Value] variant, `Flag`, or
/// `Gradient`. When a variant is given, the appropriate value will be inserted into `config`, and
/// the next argument is the value from the [Args] struct to be inserted. `Gradient` is a special
/// case where the type of the value is a [crate::settings::Gradient], with the next argument being
/// the value to be inserted. `Flag` is a special case for boolean flags given as one of two
/// command line flags. The next two arguments are the "enabled", then "disabled" flag values on
/// `Args`.
///
/// The last arguments are a sequence of string literals, describing the path to the configuration
/// key being modified.
macro_rules! merge_arg {
    ($root:tt, Flag, $enable_arg:expr, $disable_arg:expr, $($field:literal),+) => {
        let arg_value = if $enable_arg {
            Some(true)
        } else if $disable_arg {
            Some(false)
        } else {
            None
        };
        merge_arg!($root, Boolean, arg_value, $( $field ),+);
    };
    ($root:tt, Gradient, $arg:expr, $($field:literal),+) => {
        // This is hacky, but "works"
        let gradient_name = std::any::type_name::<
    };
    ($root:tt, String, $arg:expr, $($field:literal),+) => {
        if let Some(arg_member) = &$arg {
            let fields = [$( $field ),+ ];
            // Split the fields list so we have the keys we need to tranverse to get to the leaf,
            // and the name of the leaf itself.
            let (parent_fields, leaf_field) = fields.split_at(fields.len() - 1);
            let leaf_field = leaf_field[0];
            let mut parent_table = &mut $root;

            for field in parent_fields {
                parent_table = parent_table.entry(*field)
                    .or_insert_with(|| Value::Table(Table::default()))
                    .as_table_mut()
                    // TODO determine the correct error type to return for this (which can occur
                    // when a config value in a file is the wrong type.)
                    .expect("The path to a field should only contain tables");
            }
            let leaf_value = Value::String(arg_member.to_owned().to_string());
            parent_table.insert(leaf_field.into(), leaf_value);
        }
    };
    ($root:tt, $value_type:tt, $arg:expr, $($field:literal),+) => {
        if let Some(arg_member) = &$arg {
            let fields = [$( $field ),+ ];
            // Split the fields list so we have the keys we need to tranverse to get to the leaf,
            // and the name of the leaf itself.
            let (parent_fields, leaf_field) = fields.split_at(fields.len() - 1);
            let leaf_field = leaf_field[0];
            let mut parent_table = &mut $root;

            for field in parent_fields {
                parent_table = parent_table.entry(*field)
                    .or_insert_with(|| Value::Table(Table::default()))
                    .as_table_mut()
                    // TODO determine the correct error type to return for this (which can occur
                    // when a config value in a file is the wrong type.)
                    .expect("The path to a field should only contain tables");
            }
            let leaf_value = Value::$value_type(
                arg_member
                    .to_owned()
                    .try_into()
                    .unwrap_or_default()
            );
            parent_table.insert(leaf_field.into(), leaf_value);
        }
    };
}

impl Args {
    pub(crate) fn apply_to_config_str(&self, config_str: &str) -> anyhow::Result<Settings> {
        let config_table: toml::value::Table = toml::from_str(config_str)?;
        self.apply_to(config_table)
    }

    pub(crate) fn apply_to(&self, mut config: Table) -> anyhow::Result<Settings> {
        // Merge in arguments to a toml::value::Table. Using the order they're defined in the
        // struct above.
        // Skip config_path, it's not a field in Settings
        merge_arg!(config, String, self.camera_kind, "camera", "kind");
        merge_arg!(config, String, self.i2c_bus, "camera", "bus");
        merge_arg!(config, Integer, self.i2c_address, "camera", "address");
        // Have to convert NonZeroU8 to u8 before it can be converted into an i64
        merge_arg!(
            config,
            Integer,
            self.frame_rate.map(|f| f.get()),
            "camera",
            "frame_rate"
        );
        merge_arg!(config, Integer, self.grid_size, "render", "grid_size");
        merge_arg!(config, String, self.temperature_units, "render", "units");
        merge_arg!(config, String, self.colors, "render", "colors");
        merge_arg!(config, String, self.listen_address, "streams", "address");
        merge_arg!(config, Integer, self.listen_port, "streams", "port");
        merge_arg!(
            config,
            Flag,
            self.enable_mjpeg,
            self.disable_mjpeg,
            "streams",
            "mjpeg",
            "enabled"
        );
        merge_arg!(config, String, self.mqtt_name, "mqtt", "name");
        // The MQTT username and password are allowed to be empty strings, in which case they are
        // interpreted as `None`
        let mqtt_username = self.mqtt_username.as_deref().and_then(empty_to_none);
        let mqtt_password = self.mqtt_password.as_deref().and_then(empty_to_none);
        merge_arg!(config, String, mqtt_username, "mqtt", "username");
        merge_arg!(config, String, mqtt_password, "mqtt", "password");
        merge_arg!(config, String, self.mqtt_server, "mqtt", "server");
        merge_arg!(
            config,
            Flag,
            self.enable_home_assistant,
            self.disable_home_assistant,
            "mqtt",
            "home_assistant",
            "enabled"
        );
        #[cfg(feature = "mock_camera")]
        {
            merge_arg!(
                config,
                String,
                self.path.as_deref().map(|p| p.to_string_lossy()),
                "camera",
                "path"
            );
            merge_arg!(
                config,
                String,
                self.mock_repeat_mode,
                "camera",
                "repeat_mode"
            );
        }
        // Use the updated table to deserialize from
        Settings::deserialize(Value::Table(config)).map_err(anyhow::Error::from)
    }
}

fn empty_to_none(s: &str) -> Option<&str> {
    if s == "" {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod test {
    use crate::camera::{Bus, CameraSettings};
    use crate::mqtt::MqttSettings;
    use crate::occupancy::Threshold;
    use crate::temperature::Temperature;

    use super::{Args, Settings};

    fn expected_config() -> Settings {
        Settings {
            camera: CameraSettings::GridEye {
                bus: Bus::Number(9),
                address: amg88::Address::Low,
                frame_rate: amg88::FrameRateValue::Fps10,
                common: Default::default(),
            },
            streams: Default::default(),
            render: Default::default(),
            tracker: Default::default(),
            mqtt: MqttSettings {
                name: "Testing Name".to_string(),
                username: Default::default(),
                password: Default::default(),
                server: "mqtt://mqtt.invalid".parse().unwrap(),
                keep_alive: Default::default(),
                home_assistant: Default::default(),
            },
        }
    }

    #[test]
    fn minimal_no_args() -> anyhow::Result<()> {
        let source = r#"
        [camera]
        kind = "grideye"
        bus = 9
        address = 0x68
        [mqtt]
        name = "Testing Name"
        server = "mqtt://mqtt.invalid"
        "#;
        let config: Settings = toml::from_str(source)?;
        assert_eq!(config, expected_config());
        Ok(())
    }

    #[test]
    fn minimal_kind_arg() -> anyhow::Result<()> {
        let source = r#"
        [camera]
        bus = 9
        address = 0x68
        [mqtt]
        name = "Testing Name"
        server = "mqtt://mqtt.invalid"
        "#;
        let args = Args {
            camera_kind: Some("grideye".to_string()),
            ..Args::default()
        };

        let config = args.apply_to_config_str(&source)?;
        assert_eq!(config, expected_config());
        Ok(())
    }

    #[test]
    fn minimal_address_arg() -> anyhow::Result<()> {
        let source = r#"
        [camera]
        kind = "grideye"
        bus = 9
        [mqtt]
        name = "Testing Name"
        server = "mqtt://mqtt.invalid"
        "#;
        let args = Args {
            i2c_address: Some(0x68),
            ..Args::default()
        };
        let config = args.apply_to_config_str(&source)?;
        assert_eq!(config, expected_config());
        Ok(())
    }

    #[test]
    fn minimal_bus_arg() -> anyhow::Result<()> {
        let source = r#"
        [camera]
        kind = "grideye"
        address = 0x68
        [mqtt]
        name = "Testing Name"
        server = "mqtt://mqtt.invalid"
        "#;
        let args = Args {
            i2c_bus: Some(Bus::Number(9)),
            ..Args::default()
        };
        let config = args.apply_to_config_str(&source)?;
        assert_eq!(config, expected_config(), "");
        Ok(())
    }

    #[test]
    fn minimal_mqtt_name_arg() -> anyhow::Result<()> {
        let source = r#"
        [camera]
        kind = "grideye"
        bus = 9
        address = 0x68
        [mqtt]
        server = "mqtt://mqtt.invalid"
        "#;
        let args = Args {
            mqtt_name: Some("Testing Name".to_string()),
            ..Args::default()
        };
        let config = args.apply_to_config_str(&source)?;
        assert_eq!(config, expected_config());
        Ok(())
    }
    #[test]
    fn minimal_mqtt_server_arg() -> anyhow::Result<()> {
        let source = r#"
        [camera]
        kind = "grideye"
        bus = 9
        address = 0x68
        [mqtt]
        name = "Testing Name"
        "#;
        let args = Args {
            mqtt_server: Some("mqtt://mqtt.invalid".parse().unwrap()),
            ..Args::default()
        };
        let config = args.apply_to_config_str(&source)?;
        assert_eq!(config, expected_config());
        Ok(())
    }

    #[test]
    fn only_args() {
        let args = Args {
            camera_kind: Some("grideye".to_string()),
            i2c_address: Some(0x68),
            i2c_bus: Some(Bus::Number(9)),
            mqtt_name: Some("Testing Name".to_string()),
            mqtt_server: Some("mqtt://mqtt.invalid".parse().unwrap()),
            ..Args::default()
        };
        let config = args.apply_to_config_str(&"");
        assert!(
            config.is_ok(),
            "Parsing an empty config failed: {:?}",
            config
        );
        let config = config.unwrap();
        assert_eq!(config, expected_config());
    }

    #[test]
    fn minimal_empty_sections() -> anyhow::Result<()> {
        let source = r#"
        [camera]
        kind = "grideye"
        bus = 9
        address = 0x68
        [streams]
        [streams.mjpeg]
        [render]
        [tracker]
        [mqtt]
        name = "Testing Name"
        server = "mqtt://mqtt.invalid"
        [mqtt.home_assistant]
        "#;
        let config: Settings = toml::from_str(source)?;
        assert_eq!(config, expected_config());
        Ok(())
    }

    // Test setting just one field of each section. The rest of the parsing for those fields will
    // be done in their respective modules.
    #[test]
    fn non_default_subsections() -> anyhow::Result<()> {
        let source = r#"
        [camera]
        kind = "grideye"
        bus = 9
        address = 0x68
        [streams]
        mjpeg.enabled = true
        [render]
        grid_size = 42
        [tracker]
        threshold = 7
        [mqtt]
        name = "Testing Name"
        server = "mqtt://mqtt.invalid"
        [mqtt.home_assistant]
        topic = "testing_topic"
        "#;
        let mut expected = expected_config();
        expected.streams.mjpeg.enabled = true;
        expected.render.grid_size = 42;
        expected.tracker.threshold = Threshold::Static(Temperature::Celsius(7f32));
        expected.mqtt.home_assistant.topic = "testing_topic".to_string();
        let config: Settings = toml::from_str(source)?;
        assert_eq!(config, expected);
        Ok(())
    }
}
