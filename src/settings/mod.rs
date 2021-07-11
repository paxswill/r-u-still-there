// SPDX-License-Identifier: GPL-3.0-or-later
use std::fmt;

use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, MapAccess, Visitor};
use tracing::{info, warn};

mod cli;
pub(crate) mod gradient;
mod tracker;

use crate::camera::{CameraKind, CameraSettings, CameraSettingsArgs, I2cSettings};
use crate::mqtt::{MqttSettings, MqttSettingsArgs};
use crate::render::RenderSettings;
use crate::stream::StreamSettings;
pub(crate) use cli::Args;
pub(crate) use tracker::TrackerSettings;

#[derive(Debug, PartialEq)]
pub(crate) struct Settings {
    /// Camera-specific settings.
    pub(crate) camera: CameraSettings,

    /// Settings related to the HTTP server for the video streams.
    pub(crate) streams: StreamSettings,

    /// Settings related to how the data is rendered for the video streams.
    pub(crate) render: RenderSettings,

    /// Occupancy tracker settings.
    pub(crate) tracker: TrackerSettings,

    /// MQTT server connection settings.
    pub(crate) mqtt: MqttSettings,
}

// Manually implementing Deserialize so it can use DeserializeSeed, so it can pass down Args to
// those settings structs that can use them.
#[derive(Debug)]
struct SettingsArgs<'a>(&'a Args);

impl<'a> SettingsArgs<'a> {
    pub(crate) fn new(args: &'a Args) -> Self {
        Self(args)
    }
}

const SETTINGS_FIELDS: &[&str] = &["camera", "streams", "render", "tracker", "mqtt"];

impl<'de, 'a> DeserializeSeed<'de> for SettingsArgs<'a> {
    type Value = Settings;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field<'a> {
            Camera,
            Streams,
            Render,
            Tracker,
            Mqtt,
            Unknown(&'a str),
        }

        struct SettingsVisitor<'a>(&'a Args);

        impl<'de, 'a> Visitor<'de> for SettingsVisitor<'a> {
            type Value = Settings;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an r-u-still-there configuration structure")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut camera = None;
                let mut streams = None;
                let mut render = None;
                let mut tracker = None;
                let mut mqtt = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Camera => {
                            if camera.is_some() {
                                return Err(de::Error::duplicate_field("camera"));
                            }
                            camera = Some(map.next_value_seed(CameraSettingsArgs::new(self.0))?);
                        }
                        Field::Streams => {
                            if streams.is_some() {
                                return Err(de::Error::duplicate_field("streams"));
                            }
                            streams = Some(map.next_value()?);
                        }
                        Field::Render => {
                            if render.is_some() {
                                return Err(de::Error::duplicate_field("render"));
                            }
                            render = Some(map.next_value()?);
                        }
                        Field::Tracker => {
                            if tracker.is_some() {
                                return Err(de::Error::duplicate_field("tracker"));
                            }
                            tracker = Some(map.next_value()?);
                        }
                        Field::Mqtt => {
                            if mqtt.is_some() {
                                return Err(de::Error::duplicate_field("mqtt"));
                            }
                            mqtt = Some(map.next_value_seed(MqttSettingsArgs(self.0))?);
                        }
                        Field::Unknown(k) => {
                            warn!("Unknown top-level field: {}", k);
                        }
                    }
                }
                let camera = camera
                    .ok_or_else(|| de::Error::missing_field("camera"))
                    .or_else(|err| {
                        info!("Missing 'camera' key in config file, checking arguments for camera configuration");
                        // At this point, there is *no* camera configuration in the config file, so
                        // everything *better* be in the args.
                        let args = self.0;
                        match args.camera_kind.as_deref() {
                            Some("grideye") => {
                                if let (Some(bus), Some(address)) = (args.i2c_bus.as_ref(), args.i2c_address) {
                                    let bus = bus.clone();
                                    let camera_kind = CameraKind::GridEye(I2cSettings { bus, address });
                                    Ok(CameraSettings::new(camera_kind))
                                } else {
                                    Err(err)
                                }
                            }
                            Some(_) | None => {
                                Err(err)
                            }
                        }
                    })?;
                let streams = streams.unwrap_or_default();
                let render = render.unwrap_or_default();
                let tracker = tracker.unwrap_or_default();
                let mqtt = mqtt
                    .ok_or_else(|| de::Error::missing_field("mqtt"))
                    .or_else(|err| {
                        // Like with camera above, everything required for MQTT needs to be in CLI
                        // arguments if we've reached this point. Only the name and server are
                        // absolutely required to be present, and `merge_args` will ensure any
                        // other settings in the arguments are applied.
                        info!("Missing 'mqtt' key in config file, checking arguments for MQTT configuration");
                        match (self.0.mqtt_name.as_deref(), self.0.mqtt_server.as_ref()) {
                            (Some(name), Some(server)) => {
                                Ok(MqttSettings::new(name, server))
                            }
                            _ => {
                                Err(err)
                            }
                        }
                    })?;
                Ok(Settings {
                    camera,
                    streams,
                    render,
                    tracker,
                    mqtt,
                })
            }
        }

        let visitor = SettingsVisitor(self.0);
        deserializer.deserialize_struct("Settings", SETTINGS_FIELDS, visitor)
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let args = Args::default();
        SettingsArgs(&args).deserialize(deserializer)
    }
}

/// DRY macro for merging in optional CLI arguments.
///
/// It handles three simple kinds of argument that occur in the [Args] struct. The first two handle
/// the case where a field needs to be checked if it is [Some], and if it is assign that value to a
/// field on a different struct. The difference in the two cases are that the first case handles
/// types that are [Copy], while the second handles [Clone]-able types.
///
///     merge_arg!(args.source.copyable, self.destination.field);
///     merge_arg!(clone args.source.clonable, self.destination.field);
///
/// The third case is for boolean values. These are implemented on [Args] as a pair of mutually
/// exclusive flags, meaning up to one may be true. `merge_args!` checks both flags, setting a
/// destination field `true` or `false` depending on which flag is set.
///
///     merge_arg!(args.enable_flag, args.disable_flag, self.is_enabled);
macro_rules! merge_arg {
    ($arg:expr, $dest:expr) => {
        if let Some(arg_member) = $arg {
            $dest = arg_member;
        }
    };
    (clone $arg:expr, $dest:expr) => {
        if let Some(arg_member) = &$arg {
            $dest = arg_member.clone();
        }
    };
    ($enable_arg:expr, $disable_arg:expr, $dest:expr) => {
        if $enable_arg {
            $dest = true;
        } else if $disable_arg {
            $dest = false;
        }
    };
}

impl Settings {
    pub(crate) fn from_str_with_args(
        config_str: &str,
        args: &Args,
    ) -> Result<Self, toml::de::Error> {
        let mut deserializer = toml::de::Deserializer::new(config_str);
        let mut settings = SettingsArgs::new(&args).deserialize(&mut deserializer)?;
        deserializer.end()?;
        settings.merge_args(&args);
        Ok(settings)
    }

    /// Merge in values given on the command line into an existing [Settings].
    fn merge_args(&mut self, args: &Args) {
        // Don't need to merge in camera kind, i2c bus or i2c address as those are pulled in by
        // CameraSettings when using the CameraSettingsArgs deserializer
        if let Some(frame_rate) = args.frame_rate {
            self.camera.set_frame_rate(frame_rate)
        }
        merge_arg!(args.grid_size, self.render.grid_size);
        if let Some(units) = args.temperature_units {
            self.render.units = Some(units);
        }
        merge_arg!(args.colors, self.render.colors);
        merge_arg!(args.listen_address, self.streams.address);
        merge_arg!(args.listen_port, self.streams.port);
        // Only one of enable_mjpeg or disable_mjpeg can be true at a time, but it is is possible
        // for both to be false. In the case where both are false, the value read from defaults (or
        // a config file) is preserved.
        merge_arg!(
            args.enable_mjpeg,
            args.disable_mjpeg,
            self.streams.mjpeg.enabled
        );
        merge_arg!(clone args.mqtt_name, self.mqtt.name);
        // The MQTT username and password are allowed to be empty strings, in which case they are
        // interpreted as `None`
        match args.mqtt_username.as_deref() {
            Some("") => self.mqtt.username = None,
            Some(username) => self.mqtt.username = Some(username.to_string()),
            None => (),
        }
        match args.mqtt_password.as_deref() {
            Some("") => self.mqtt.password = None,
            Some(password) => self.mqtt.password = Some(password.into()),
            None => (),
        }
        // Same situation for *_home_assistant as*_mjpeg.
        merge_arg!(
            args.enable_home_assistant,
            args.disable_home_assistant,
            self.mqtt.home_assistant.enabled
        );
    }
}

#[cfg(test)]
mod test {
    use crate::camera::{Bus, CameraKind, CameraSettings, I2cSettings};
    use crate::mqtt::MqttSettings;
    use crate::occupancy::Threshold;
    use crate::temperature::Temperature;

    use super::cli::Args;
    use super::Settings;

    fn expected_config() -> Settings {
        Settings {
            camera: CameraSettings::new(CameraKind::GridEye(I2cSettings {
                bus: Bus::Number(9),
                address: 0x68,
            })),
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
        let config = Settings::from_str_with_args(source, &args)?;
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
        let config = Settings::from_str_with_args(source, &args)?;
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
        let config = Settings::from_str_with_args(source, &args)?;
        assert_eq!(config, expected_config());
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
        let config = Settings::from_str_with_args(source, &args)?;
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
        let config = Settings::from_str_with_args(source, &args)?;
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
        let config = Settings::from_str_with_args("", &args);
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

    // Test setting jsut one field of each section. The rest of the parsing for those fields will
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
