// SPDX-License-Identifier: GPL-3.0-or-later
use serde::de::{self, Deserialize, Deserializer, MapAccess, Visitor};
use serde_repr::Deserialize_repr;

use std::fmt;

use super::i2c::{Bus, I2cSettings};

// This enum is purely used to restrict the acceptable values for rotation
#[derive(Deserialize_repr, PartialEq, Debug)]
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
pub struct CameraOptions {
    rotation: u16,
    mirror: bool,
    frame_rate: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraSettings<'a> {
    GridEye {
        i2c: I2cSettings<'a>,
        options: CameraOptions,
    },
}

const CAMERA_KINDS: &'static [&'static str] = &["grideye"];

const CAMERA_FIELDS: &'static [&'static str] =
    &["kind", "bus", "address", "rotation", "mirror", "frame_rate"];

// Manually implementing Derserialize as there isn't a way to derive a flattened enum
// implementation.
impl<'de> Deserialize<'de> for CameraSettings<'de> {
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
            Mirror,
            FrameRate,
            Kind,
            Unknown(&'a str),
        }

        struct CameraVisitor;

        impl<'de> Visitor<'de> for CameraVisitor {
            type Value = CameraSettings<'de>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("enum CameraSettings")
            }

            fn visit_map<V>(self, mut map: V) -> Result<CameraSettings<'de>, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut bus = None;
                let mut address = None;
                let mut rotation = None;
                let mut mirror = None;
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
                        Field::Mirror => {
                            if mirror.is_some() {
                                return Err(de::Error::duplicate_field("mirror"));
                            }
                            mirror = Some(map.next_value()?);
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
                let rotation: Rotation = rotation.unwrap_or(Rotation::default());
                let mirror = mirror.unwrap_or(false);
                // True options skipped (frame_rate)
                let i2c = I2cSettings::<'de> { bus, address };
                let options = CameraOptions {
                    rotation: rotation as u16,
                    mirror,
                    frame_rate,
                };
                match kind {
                    "grideye" => Ok(CameraSettings::GridEye { i2c, options }),
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
    use super::{Bus, CameraOptions, CameraSettings, I2cSettings};

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
            "mirror = true",
            "frame_rate = 7",
        ];
        let full = lines.join("\n").to_string();
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
            options: CameraOptions {
                rotation: 0,
                mirror: false,
                frame_rate: None,
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
            options: CameraOptions {
                rotation: 0,
                mirror: false,
                frame_rate: None,
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
        mirror = true
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
            options: CameraOptions {
                rotation: 180,
                mirror: true,
                frame_rate: Some(7),
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
        mirror = true
        frame_rate = 7
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Path("1"),
                address: 30,
            },
            options: CameraOptions {
                rotation: 180,
                mirror: true,
                frame_rate: Some(7),
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
            "mirror": true,
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
            options: CameraOptions {
                rotation: 180,
                mirror: true,
                frame_rate: Some(7),
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
            "mirror": true,
            "frame_rate": 7
        }"#;
        let parsed = serde_json::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse JSON: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            i2c: I2cSettings {
                bus: Bus::Path("1"),
                address: 30,
            },
            options: CameraOptions {
                rotation: 180,
                mirror: true,
                frame_rate: Some(7),
            },
        };
        assert_eq!(parsed, expected);
    }
}
