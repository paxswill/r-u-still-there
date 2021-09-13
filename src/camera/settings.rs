// SPDX-License-Identifier: GPL-3.0-or-later
use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU8;
use std::path::PathBuf;

use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, MapAccess, Visitor};
use serde_repr::Deserialize_repr;
use tracing::{debug, error};

use super::I2cSettings;
use crate::settings::Args;

// This enum is purely used to restrict the acceptable values for rotation
#[derive(Clone, Copy, Deserialize_repr, PartialEq, Debug)]
#[repr(u16)]
pub(crate) enum Rotation {
    Zero = 0,
    Ninety = 90,
    OneEighty = 180,
    TwoSeventy = 270,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum CameraKind {
    GridEye(I2cSettings),
    Mlx90640(I2cSettings),
    Mlx90641(I2cSettings),
    #[cfg(feature = "mock_camera")]
    #[serde(rename = "mock")]
    MockCamera(PathBuf),
}

#[derive(Clone, Debug)]
pub(crate) struct CameraSettings {
    pub(crate) kind: CameraKind,

    pub(crate) rotation: Rotation,

    pub(crate) flip_horizontal: bool,

    pub(crate) flip_vertical: bool,

    frame_rate: Option<NonZeroU8>,

    /// If `Some`, [measurements][crate::camera::Measurement] should be saved to this path.
    pub(crate) path: Option<PathBuf>,
}

impl Default for Rotation {
    fn default() -> Self {
        Self::Zero
    }
}

impl CameraKind {
    /// Get the default frame rate for a camera module
    fn default_frame_rate(&self) -> u8 {
        match self {
            CameraKind::GridEye(_) => 10,
            CameraKind::Mlx90640(_) => 2,
            CameraKind::Mlx90641(_) => 2,

            // NOTE: The frame rate for the mock camera is a multiplier for the recorded speed.
            #[cfg(feature = "mock_camera")]
            CameraKind::MockCamera(_) => 1,
        }
    }
}

const CAMERA_KINDS: &[&str] = &["grideye"];

const CAMERA_FIELDS: &[&str] = &[
    "bus",
    "address",
    "rotation",
    "flip_horizontal",
    "flip_vertical",
    "frame_rate",
    "kind",
    "path",
];

pub(crate) struct CameraSettingsArgs<'a>(&'a Args);

impl<'a> CameraSettingsArgs<'a> {
    pub(crate) fn new(args: &'a Args) -> Self {
        Self(args)
    }
}

// Manually implementing Derserialize as there isn't a way to derive DeserializeSeed
impl<'de, 'a> DeserializeSeed<'de> for CameraSettingsArgs<'a> {
    type Value = CameraSettings;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
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
            Path,
            Unknown(&'a str),
        }

        struct CameraVisitor<'a>(&'a Args);

        impl<'de, 'a> Visitor<'de> for CameraVisitor<'a> {
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
                let mut kind: Option<Cow<'_, str>> = None;
                let mut path: Option<PathBuf> = None;
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
                        Field::Path => {
                            if path.is_some() {
                                return Err(de::Error::duplicate_field("path"));
                            }
                            path = Some(map.next_value()?);
                        }
                        Field::Unknown(_) => {}
                    }
                }
                // kind is required, and can be given either by being deserialized, or as a CLI
                // argument in Args. There may also be other required fields depending on the value
                // of kind.
                let kind = self
                    .0
                    .camera_kind
                    .as_ref()
                    .map(|kind| Cow::Owned(kind.clone()))
                    .or(kind)
                    .ok_or_else(|| de::Error::missing_field("kind"))?;
                // Fields with defaults
                let rotation: Rotation = rotation.unwrap_or_default();
                let flip_horizontal = flip_horizontal.unwrap_or(false);
                let flip_vertical = flip_vertical.unwrap_or(false);
                let frame_rate = frame_rate.unwrap_or_default();
                // bus and address are required depending on the kind of camera, so they may come
                // from being deserialized or from CLI args.
                debug!("I2C bus from config: {:?}", bus);
                debug!("I2C bus from CLI: {:?}", self.0.i2c_bus);
                let bus = self.0.i2c_bus.as_ref().cloned().or(bus);
                debug!("I2C address from config: {:?}", address);
                debug!("I2C address from CLI: {:?}", self.0.i2c_address);
                let address = self.0.i2c_address.or(address);
                // Path is required for mock cameras, so it's handled in here as well.
                path = self.0.path.as_ref().cloned().or(path);
                let kind = match kind.as_ref() {
                    "grideye" => {
                        debug!(camera_kind = %kind, "using a GridEYE camera");
                        let bus = bus.ok_or_else(|| de::Error::missing_field("bus"))?;
                        let address = address.ok_or_else(|| de::Error::missing_field("address"))?;
                        CameraKind::GridEye(I2cSettings { bus, address })
                    }
                    "mlx90640" => {
                        debug!(camera_kind = %kind, "using a MLX90640");
                        let bus = bus.ok_or_else(|| de::Error::missing_field("bus"))?;
                        let address = address.ok_or_else(|| de::Error::missing_field("address"))?;
                        CameraKind::Mlx90640(I2cSettings { bus, address })
                    }
                    "mlx90641" => {
                        debug!(camera_kind = %kind, "using a MLX90641");
                        let bus = bus.ok_or_else(|| de::Error::missing_field("bus"))?;
                        let address = address.ok_or_else(|| de::Error::missing_field("address"))?;
                        CameraKind::Mlx90641(I2cSettings { bus, address })
                    }
                    #[cfg(feature = "mock_camera")]
                    "mock" => {
                        // The mock camera's field will just be ignored by the other cameras
                        // (until there's a camera that takes a path).
                        debug!(camera_kind = %kind, "using the mock camera");
                        let path = path
                            // When `path` is Some, `Pipeline` adds a subscriber to the camera
                            // stream that saves all measurement data to a file. Because a mock
                            // camera is supposed to be *reading* from that file, `path` needs to
                            // be None.
                            .take()
                            .ok_or_else(|| de::Error::missing_field("path"))?;
                        CameraKind::MockCamera(path)
                    }
                    _ => {
                        error!(camera_kind = %kind, "unknown camera kind");
                        return Err(de::Error::unknown_variant(kind.as_ref(), CAMERA_KINDS));
                    }
                };
                Ok(CameraSettings {
                    kind,
                    rotation,
                    flip_horizontal,
                    flip_vertical,
                    frame_rate,
                    path,
                })
            }
        }
        let visitor = CameraVisitor(self.0);
        // Just a "hint" that this is a struct when it's actually deserializing an enum.
        deserializer.deserialize_struct("CameraSettings", CAMERA_FIELDS, visitor)
    }
}

impl<'de> Deserialize<'de> for CameraSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let args = Args::default();
        CameraSettingsArgs(&args).deserialize(deserializer)
    }
}

impl CameraSettings {
    pub(crate) fn new(kind: CameraKind) -> Self {
        Self {
            kind: kind,
            rotation: Rotation::default(),
            flip_horizontal: false,
            flip_vertical: false,
            frame_rate: None,
            path: None,
        }
    }

    /// Get the configured frame rate.
    pub(crate) fn frame_rate(&self) -> u8 {
        self.frame_rate
            .map(NonZeroU8::get)
            .unwrap_or_else(|| self.kind.default_frame_rate())
    }

    /// Set the requested frame rate in the configuration.
    ///
    /// Note, this does not actually set the frame rate itself, it sets the *configured* frame
    /// rate. the actual frame rate is determined in [shared_camera::Camera::frame_stream].
    pub(crate) fn set_frame_rate(&mut self, frame_rate: NonZeroU8) {
        self.frame_rate = Some(frame_rate)
    }
}

impl PartialEq for CameraSettings {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.rotation == other.rotation
            && self.flip_horizontal == other.flip_horizontal
            && self.flip_vertical == other.flip_vertical
            && self.frame_rate() == other.frame_rate()
    }
}

#[cfg(test)]
mod de_tests {
    // Missing pytest's parameterized tests here.
    use std::num::NonZeroU8;
    use std::path::PathBuf;

    use crate::camera::{Bus, I2cSettings};

    use super::{CameraKind, CameraSettings, Rotation};

    #[test]
    fn bus_from_num() {
        assert_eq!(Bus::from(0), Bus::Number(0))
    }

    #[test]
    fn bus_num_from_decimal_string() {
        let bus: Result<Bus, _> = "0".parse();
        assert!(bus.is_ok());
        let bus = bus.unwrap();
        assert_eq!(bus, Bus::Number(0))
    }

    #[test]
    fn bus_num_from_hex_string() {
        let bus: Result<Bus, _> = "0x68".parse();
        assert!(bus.is_ok());
        let bus = bus.unwrap();
        assert_eq!(bus, Bus::Number(0x68))
    }

    #[test]
    fn bus_path_from_string() {
        let bus: Result<Bus, _> = "/dev/i2c-0".parse();
        assert!(bus.is_ok());
        let bus = bus.unwrap();
        assert_eq!(bus, Bus::Path(PathBuf::from("/dev/i2c-0")));
    }

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
        let expected = CameraSettings {
            kind: CameraKind::GridEye(I2cSettings {
                bus: Bus::Number(1),
                address: 30,
            }),
            rotation: Rotation::Zero,
            flip_horizontal: false,
            flip_vertical: false,
            frame_rate: Some(NonZeroU8::new(10).unwrap()),
            path: None,
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_full_bus_num() {
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
        let expected = CameraSettings {
            kind: CameraKind::GridEye(I2cSettings {
                bus: Bus::Number(1),
                address: 30,
            }),
            rotation: Rotation::OneEighty,
            flip_horizontal: true,
            flip_vertical: true,
            frame_rate: Some(NonZeroU8::new(7).unwrap()),
            path: None,
        };
        assert_eq!(parsed, expected);
    }

    /// Test that the path field is preserved for cameras other than `MockCamera`.
    #[test]
    fn non_mock_path() {
        let source = r#"
        kind = "mlx90640"
        bus = 1
        address = 0x33
        frame_rate = 8
        path = "/foo/bar/baz.bin"
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings {
            kind: CameraKind::Mlx90640(I2cSettings {
                bus: Bus::Number(1),
                address: 0x33,
            }),
            rotation: Rotation::default(),
            flip_horizontal: false,
            flip_vertical: false,
            frame_rate: Some(NonZeroU8::new(8).unwrap()),
            path: Some("/foo/var/baz.bin".into()),
        };
        assert_eq!(parsed, expected);
    }

    /// Ensure that `MockCamera` clears `path`, and that the value specified in path is used for
    /// the field within `MockCamera`. Also testing that `bus` and `address` are ignored for
    /// `MockCamera` (but can still be present).
    #[cfg(feature = "mock_camera")]
    #[test]
    fn mock_camera() {
        let source = r#"
        kind = "mock"
        bus = 1
        address = 0x33
        frame_rate = 3
        path = "/tmp/qux.bin"
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings {
            kind: CameraKind::MockCamera("/tmp/qux.bin".into()),
            rotation: Rotation::default(),
            flip_horizontal: false,
            flip_vertical: false,
            frame_rate: Some(NonZeroU8::new(3).unwrap()),
            path: None,
        };
        assert_eq!(parsed, expected);
    }
}
