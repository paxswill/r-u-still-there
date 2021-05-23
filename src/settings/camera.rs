// SPDX-License-Identifier: GPL-3.0-or-later
use serde::de::{self, Deserialize, Deserializer, MapAccess, Visitor};
use serde_repr::Deserialize_repr;

use std::fmt;
use std::time::Duration;

use super::i2c::I2cSettings;

// This enum is purely used to restrict the acceptable values for rotation
#[derive(Clone, Copy, Deserialize_repr, PartialEq, Debug)]
#[repr(u16)]
pub enum Rotation {
    Zero = 0,
    Ninety = 90,
    OneEighty = 180,
    TwoSeventy = 270,
}

impl Default for Rotation {
    fn default() -> Self {
        Self::Zero
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommonOptions {
    pub rotation: Rotation,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub frame_rate: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CameraSettings {
    GridEye {
        i2c: I2cSettings,
        options: CommonOptions,
    },
}

const CAMERA_KINDS: &[&str] = &["grideye"];

const CAMERA_FIELDS: &[&str] = &[
    "kind",
    "bus",
    "address",
    "rotation",
    "flip_horizontal",
    "flip_vertical",
    "frame_rate",
];

// Manually implementing Derserialize as there isn't a way to derive a flattened enum
// implementation.
impl<'de> Deserialize<'de> for CameraSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field<'a> {
            Bus,
            Address,
            Rotation,
            FlipHorizontal,
            FlipVertical,
            FrameRate,
            Kind,
            Unknown(&'a str),
        }

        struct CameraVisitor;

        impl<'de> Visitor<'de> for CameraVisitor {
            type Value = CameraSettings;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("enum CameraSettings")
            }

            fn visit_map<V>(self, mut map: V) -> Result<CameraSettings, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut bus = None;
                let mut address = None;
                let mut rotation = None;
                let mut flip_horizontal = None;
                let mut flip_vertical = None;
                let mut frame_rate = None;
                let mut kind = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Bus => {
                            if bus.is_some() {
                                return Err(de::Error::duplicate_field("bus"));
                            }
                            bus = Some(map.next_value()?);
                        }
                        Field::Address => {
                            if address.is_some() {
                                return Err(de::Error::duplicate_field("address"));
                            }
                            address = Some(map.next_value()?);
                        }
                        Field::Rotation => {
                            if rotation.is_some() {
                                return Err(de::Error::duplicate_field("rotation"));
                            }
                            rotation = Some(map.next_value()?);
                        }
                        Field::FlipHorizontal => {
                            if flip_horizontal.is_some() {
                                return Err(de::Error::duplicate_field("flip_horizontal"));
                            }
                            flip_horizontal = Some(map.next_value()?);
                        }
                        Field::FlipVertical => {
                            if flip_vertical.is_some() {
                                return Err(de::Error::duplicate_field("flip_vertical"));
                            }
                            flip_vertical = Some(map.next_value()?);
                        }
                        Field::FrameRate => {
                            if frame_rate.is_some() {
                                return Err(de::Error::duplicate_field("frame_rate"));
                            }
                            frame_rate = Some(map.next_value()?);
                        }
                        Field::Kind => {
                            if kind.is_some() {
                                return Err(de::Error::duplicate_field("kind"));
                            }
                            kind = Some(map.next_value()?);
                        }
                        Field::Unknown(_) => {}
                    }
                }
                // Required fields
                let bus = bus.ok_or_else(|| de::Error::missing_field("bus"))?;
                let address = address.ok_or_else(|| de::Error::missing_field("address"))?;
                let kind = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
                // Fields with defaults
                let rotation: Rotation = rotation.unwrap_or_default();
                let flip_horizontal = flip_horizontal.unwrap_or(false);
                let flip_vertical = flip_vertical.unwrap_or(false);
                // Minimal check of frame_rate. Variants are expected to set frame_rate to an
                // actual value themselves below.
                // This can be simplified if the `option_result_contains` API gets standardized.
                if frame_rate.is_some() {
                    let frame_rate = frame_rate.clone().unwrap();
                    if frame_rate == 0 {
                        return Err(serde::de::Error::invalid_value(
                            serde::de::Unexpected::Unsigned(0),
                            &"a frame rate greater than 0",
                        ));
                    }
                }
                let i2c = I2cSettings { bus, address };
                let options = CommonOptions {
                    rotation,
                    flip_horizontal,
                    flip_vertical,
                    frame_rate: frame_rate.clone().unwrap_or(1),
                };
                match kind {
                    "grideye" => {
                        // The GridEYE only supports up to 10 FPS
                        let frame_rate = match frame_rate {
                            None => Ok(10),
                            Some(n @ 1..=10) => Ok(n),
                            Some(n) => Err(de::Error::invalid_value(
                                de::Unexpected::Unsigned(n as u64),
                                &"a frame rate between 1 and 10",
                            )),
                        }?;
                        // No base update syntax for enums :(
                        let options = CommonOptions {
                            rotation: options.rotation,
                            flip_horizontal: options.flip_horizontal,
                            flip_vertical: options.flip_vertical,
                            frame_rate,
                        };
                        Ok(CameraSettings::GridEye { i2c, options })
                    }
                    _ => Err(de::Error::unknown_variant(kind, CAMERA_KINDS)),
                }
            }
        }
        // Just a "hint" that this is a struct when it's actually deserializing an enum.
        deserializer.deserialize_struct("CameraSettings", CAMERA_FIELDS, CameraVisitor)
    }
}

#[cfg(test)]
mod de_tests {
    // I'm not sure I need to include both TOML and JSON test cases, but v0v
    // Also missing pytest's parameterized tests here.
    use super::{CameraSettings, CommonOptions, Rotation};
    use crate::settings::i2c::{Bus, I2cSettings};

    #[test]
    fn error_0_frame_rate() {
        let source = r#"
        kind = "grideye"
        bus = 1
        address = 30
        frame_rate = 0
        "#;
        let parsed: Result<CameraSettings, _> = toml::from_str(source);
        assert!(
            parsed.is_err(),
            "Accepted invalid frame_rate value:\n{}",
            source
        );
    }

    #[test]
    fn error_invalid_rotation() {
        let source = r#"
        kind = "grideye"
        bus = 1
        address = 30
        rotation = 100
        "#;
        let parsed: Result<CameraSettings, _> = toml::from_str(source);
        assert!(
            parsed.is_err(),
            "Accepted invalid rotation value:\n{}",
            source
        );
    }

    #[test]
    fn error_bad_kind() {
        let source = r#"
        kind = "NotARealCamera"
        bus = 1
        address = 30
        "#;
        let parsed: Result<CameraSettings, _> = toml::from_str(source);
        assert!(
            parsed.is_err(),
            "Did not detect invalid camera kind in:\n{}",
            source
        );
    }

    #[test]
    fn error_extra() {
        let lines = [
            "kind = \"grideye\"",
            "bus = 1",
            "address = 30",
            "rotation = 180",
            "flip_horizontal = true",
            "flip_vertical = true",
            "frame_rate = 7",
        ];
        let full = lines.join("\n");
        for line in &lines {
            let source = full.clone() + line;
            let parsed: Result<CameraSettings, _> = toml::from_str(&source);
            assert!(
                parsed.is_err(),
                "Did not detect duplicate key in:\n{}",
                source
            );
        }
    }

    #[test]
    fn error_missing() {
        let lines = ["kind = \"grideye\"", "bus = 1", "address = 30"];
        for i in 0..lines.len() {
            let source = lines
                .iter()
                .enumerate()
                .filter(|(n, _)| *n != i)
                .map(|(_, v)| v.to_owned())
                .fold("".to_string(), |full, tail| full + "\n" + tail);
            let parsed: Result<CameraSettings, _> = toml::from_str(&source);
            assert!(
                parsed.is_err(),
                "Did not detect missing key from:\n{}",
                source
            );
        }
    }

    #[test]
    fn grideye_minimal_toml() {
        let source = r#"
        kind = "grideye"
        bus = 1
        address = 30
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Number(1),
                address: 30,
            },
            options: CommonOptions {
                rotation: Rotation::Zero,
                flip_horizontal: false,
                flip_vertical: false,
                frame_rate: 10,
            },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_minimal_json() {
        let source = r#"
        {
            "kind": "grideye",
            "bus": 1,
            "address": 30
        }"#;
        let parsed = serde_json::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse JSON: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Number(1),
                address: 30,
            },
            options: CommonOptions {
                rotation: Rotation::Zero,
                flip_horizontal: false,
                flip_vertical: false,
                frame_rate: 10,
            },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_full_toml_bus_num() {
        let source = r#"
        kind = "grideye"
        bus = 1
        address = 30
        rotation = 180
        flip_horizontal = true
        flip_vertical = true
        frame_rate = 7
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Number(1),
                address: 30,
            },
            options: CommonOptions {
                rotation: Rotation::OneEighty,
                flip_horizontal: true,
                flip_vertical: true,
                frame_rate: 7,
            },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_full_toml_bus_str() {
        let source = r#"
        kind = "grideye"
        bus = "1"
        address = 30
        rotation = 180
        flip_horizontal = true
        flip_vertical = true
        frame_rate = 7
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Path("1".to_string()),
                address: 30,
            },
            options: CommonOptions {
                rotation: Rotation::OneEighty,
                flip_horizontal: true,
                flip_vertical: true,
                frame_rate: 7,
            },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_full_json_bus_num() {
        let source = r#"
        {
            "kind": "grideye",
            "bus": 1,
            "address": 30,
            "rotation": 180,
            "flip_horizontal": true,
            "flip_vertical": true,
            "frame_rate": 7
        }"#;
        let parsed = serde_json::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse JSON: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Number(1),
                address: 30,
            },
            options: CommonOptions {
                rotation: Rotation::OneEighty,
                flip_horizontal: true,
                flip_vertical: true,
                frame_rate: 7,
            },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_full_json_bus_str() {
        let source = r#"
        {
            "kind": "grideye",
            "bus": "1",
            "address": 30,
            "rotation": 180,
            "flip_horizontal": true,
            "flip_vertical": true,
            "frame_rate": 7
        }"#;
        let parsed = serde_json::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse JSON: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Path("1".to_string()),
                address: 30,
            },
            options: CommonOptions {
                rotation: Rotation::OneEighty,
                flip_horizontal: true,
                flip_vertical: true,
                frame_rate: 7,
            },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_min_frame_rate() {
        let source = r#"
        kind = "grideye"
        bus = 1
        address = 30
        frame_rate = 1
        "#;
        let parsed: Result<CameraSettings, _> = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Number(1),
                address: 30,
            },
            options: CommonOptions {
                rotation: Rotation::Zero,
                flip_horizontal: false,
                flip_vertical: false,
                frame_rate: 1,
            },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_max_frame_rate() {
        let source = r#"
        kind = "grideye"
        bus = 1
        address = 30
        frame_rate = 10
        "#;
        let parsed: Result<CameraSettings, _> = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Number(1),
                address: 30,
            },
            options: CommonOptions {
                rotation: Rotation::Zero,
                flip_horizontal: false,
                flip_vertical: false,
                frame_rate: 10,
            },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn error_grideye_over_frame_rate() {
        let source = r#"
        kind = "grideye"
        bus = 1
        address = 30
        frame_rate = 11
        "#;
        let parsed: Result<CameraSettings, _> = toml::from_str(source);
        assert!(
            parsed.is_err(),
            "Accepted frame_rate greater than 10:\n{}",
            source
        );
    }
}

impl From<CommonOptions> for Duration {
    fn from(options: CommonOptions) -> Self {
        Self::from_millis(1000 / options.frame_rate as u64)
    }
}

impl From<CameraSettings> for I2cSettings {
    fn from(settings: CameraSettings) -> Self {
        match settings {
            CameraSettings::GridEye { i2c, options: _ } => i2c,
        }
    }
}

impl<'a> From<&'a CameraSettings> for &'a I2cSettings {
    fn from(settings: &'a CameraSettings) -> Self {
        match settings {
            CameraSettings::GridEye { i2c, options: _ } => i2c,
        }
    }
}

impl From<CameraSettings> for CommonOptions {
    fn from(settings: CameraSettings) -> Self {
        match settings {
            CameraSettings::GridEye {
                i2c: _,
                options: common_options,
            } => common_options,
        }
    }
}
