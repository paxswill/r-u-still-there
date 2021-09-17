// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::marker::PhantomData;
use std::path::PathBuf;

use anyhow::Context as _;
use linux_embedded_hal::I2cdev;
use serde::de::{Deserialize, Deserializer, Error};
use serde::ser::{Serialize, Serializer};
use serde_repr::{Deserialize_repr, Serialize_repr};

use super::thermal_camera::{self, ThermalCamera};

/// The type for the map of extra keys found in a camera config.
type ExtraMap = HashMap<String, toml::Value>;

// This enum is purely used to restrict the acceptable values for rotation.
#[derive(Clone, Copy, Deserialize_repr, Serialize_repr, PartialEq, Debug)]
#[repr(u16)]
pub(crate) enum Rotation {
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

/// Newtype wrapper around `bool` for flipping the image.
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
struct Flip(bool);

impl Default for Flip {
    fn default() -> Self {
        Flip(false)
    }
}

impl From<bool> for Flip {
    fn from(value: bool) -> Self {
        Flip(value)
    }
}

impl From<Flip> for bool {
    fn from(wrapped: Flip) -> Self {
        wrapped.0
    }
}

#[derive(Clone, Default, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct CommonCameraSettings {
    #[serde(default)]
    rotation: Rotation,

    #[serde(default)]
    flip_horizontal: Flip,

    #[serde(default)]
    flip_vertical: Flip,

    // By annotating this field with 'flatten', any unknown keys will be collected into this map.
    #[serde(default, flatten)]
    extra: ExtraMap,
}

/// Duplicating [amg88::Address] to have [serde::Deserialize] implemented for it.
///
/// This is used to validate that the address specified is one of the two valid addresses.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize_repr, Serialize_repr)]
#[serde(remote = "amg88::Address")]
#[repr(u8)]
enum GridEyeAddress {
    Low = 0x68,
    High = 0x69,
}

fn default_grideye_frame_rate() -> amg88::FrameRateValue {
    amg88::FrameRateValue::Fps10
}

struct TryFromNum<U>(PhantomData<U>);

impl<U> TryFromNum<U> {
    #[allow(dead_code)]
    pub(super) fn serialize<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        U: From<T> + Serialize + Copy,
        T: Copy,
    {
        U::from(*value).serialize(serializer)
    }

    pub(super) fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: TryFrom<U>,
        <T as TryFrom<U>>::Error: fmt::Display,
        U: Deserialize<'de>,
    {
        let value: U = U::deserialize(deserializer)?;
        T::try_from(value).map_err(|err| D::Error::custom(err))
    }
}

type TryFromU8 = TryFromNum<u8>;
type TryFromF32 = TryFromNum<f32>;

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub(crate) enum CameraSettings {
    GridEye {
        bus: super::i2c::Bus,

        #[serde(with = "TryFromU8")]
        address: amg88::Address,

        #[serde(default = "default_grideye_frame_rate", with = "TryFromU8")]
        frame_rate: amg88::FrameRateValue,

        #[serde(flatten)]
        common: CommonCameraSettings,
    },
    Mlx90640 {
        bus: super::i2c::Bus,
        address: u8,

        #[serde(default, with = "TryFromF32")]
        frame_rate: mlx9064x::FrameRate,

        #[serde(flatten)]
        common: CommonCameraSettings,
    },
    Mlx90641 {
        bus: super::i2c::Bus,
        address: u8,

        #[serde(with = "TryFromF32")]
        frame_rate: mlx9064x::FrameRate,

        #[serde(flatten)]
        common: CommonCameraSettings,
    },
    #[cfg(feature = "mock_camera")]
    #[serde(rename = "mock")]
    MockCamera {
        path: PathBuf,
        frame_rate: f32,

        #[serde(default)]
        repeat_mode: super::RepeatMode,

        #[serde(flatten)]
        common: CommonCameraSettings,
    },
}

impl CameraSettings {
    /// A list of the different camera kind identifiers
    // It'd be nice at some point to have these be automatically generated, as serde sees all this
    // information.
    #[cfg(feature = "mock_camera")]
    pub(crate) const KINDS: &'static [&'static str] = &["grideye", "mlx90640", "mlx90641", "mock"];
    #[cfg(not(feature = "mock_camera"))]
    pub(crate) const KINDS: &'static [&'static str] = &["grideye", "mlx90640", "mlx90641"];

    /// Convenience method for accessing common camera settings.
    fn common(&self) -> &CommonCameraSettings {
        match self {
            Self::GridEye { common, .. } => common,
            Self::Mlx90640 { common, .. } => common,
            Self::Mlx90641 { common, .. } => common,
            #[cfg(feature = "mock_camera")]
            Self::MockCamera { common, .. } => common,
        }
    }

    /// The requested rotation of the image.
    pub(crate) fn rotation(&self) -> Rotation {
        self.common().rotation
    }

    /// Whether the image should be flipped horizontally.
    pub(crate) fn flip_horizontal(&self) -> bool {
        self.common().flip_horizontal.into()
    }

    /// Whether the image should be flipped vertically.
    pub(crate) fn flip_vertical(&self) -> bool {
        self.common().flip_vertical.into()
    }

    /// Access any unprocessed keys from the configuration.
    pub(crate) fn extra(&self) -> &ExtraMap {
        &self.common().extra
    }

    /// If the camera is connected over I2C, this method creates the [I2cdev] for that bus.
    ///
    /// If the camera does not use I2C, this method returns `None`.
    fn i2c_bus(&self) -> Option<anyhow::Result<I2cdev>> {
        match self {
            Self::GridEye { bus, .. } => Some(bus),
            Self::Mlx90640 { bus, .. } => Some(bus),
            Self::Mlx90641 { bus, .. } => Some(bus),
            _ => None,
        }
        .map(|bus| I2cdev::try_from(bus).context("Unable to connect to I2C bus"))
    }

    pub(crate) fn frame_rate(&self) -> u8 {
        match self {
            Self::GridEye {
                frame_rate: amg88::FrameRateValue::Fps1,
                ..
            } => 1,
            Self::GridEye {
                frame_rate: amg88::FrameRateValue::Fps10,
                ..
            } => 10,
            // Clamp the 0.5 FPS frame rate for Melexis cameras to 1 until non-integer frame rates
            // are implemented.
            Self::Mlx90640 {
                frame_rate: mlx9064x::FrameRate::Half,
                ..
            } => 1,
            Self::Mlx90641 {
                frame_rate: mlx9064x::FrameRate::Half,
                ..
            } => 1,
            // Safe to truncate the floats to integers as the values are only the powers of 2, (0
            // through 6, so 2 through 64).
            Self::Mlx90640 { frame_rate, .. } => f32::from(*frame_rate) as u8,
            Self::Mlx90641 { frame_rate, .. } => f32::from(*frame_rate) as u8,
            // Just clamp the mock frame rate between 1 and u8::MAX
            #[cfg(feature = "mock_camera")]
            Self::MockCamera { frame_rate, .. } => frame_rate.max(1.0).min(u8::MAX as f32) as u8,
        }
    }

    pub(crate) fn create_camera(&self) -> anyhow::Result<Box<dyn ThermalCamera + Send>> {
        Ok(match self {
            Self::GridEye { address, .. } => Box::new(thermal_camera::GridEye::new(
                self.i2c_bus().expect("GridEye uses I2C")?,
                *address,
            )?),
            Self::Mlx90640 { address, .. } => {
                let bus = self.i2c_bus().expect("MLX90640 uses I2C")?;
                Box::new(thermal_camera::Mlx90640::new(
                    mlx9064x::Mlx90640Driver::new(bus, *address)?,
                ))
            }
            Self::Mlx90641 { address, .. } => {
                let bus = self.i2c_bus().expect("MLX90641 uses I2C")?;
                Box::new(thermal_camera::Mlx90641::new(
                    mlx9064x::Mlx90641Driver::new(bus, *address)?,
                ))
            }
            #[cfg(feature = "mock_camera")]
            Self::MockCamera {
                path, repeat_mode, ..
            } => {
                use std::io::{Read, Seek};

                use bincode::Options;

                use crate::camera::mock_camera::{MeasurementData, MockCamera};

                let extension = path.extension().map(|s| s.to_str()).flatten();
                let measurements: Vec<MeasurementData> = match extension {
                    Some("toml") => {
                        let data_string = std::fs::read_to_string(path)?;
                        toml::from_str(&data_string).map_err(anyhow::Error::from)
                    }
                    // treat everything as bincode if we don't know the extension
                    _ => {
                        let mut measurements = Vec::new();
                        let mut file = std::fs::File::open(path)?;
                        let file_size = file.metadata()?.len();
                        // These are the options async-bincode uses (but skipping the limit).
                        let bincode_options = bincode::options()
                            .with_fixint_encoding()
                            .allow_trailing_bytes();
                        while file.stream_position()? < file_size {
                            // Have to keep cloning as bincode_options would otherwise be consumed
                            let frame = bincode_options.clone().deserialize_from(file.by_ref())?;
                            measurements.push(frame);
                        }
                        Ok(measurements)
                    }
                }?;
                let mock_cam = MockCamera::new(measurements, *repeat_mode);
                Box::new(mock_cam)
            }
        })
    }
}

#[cfg(test)]
mod de_tests {
    use std::path::PathBuf;

    use crate::camera::Bus;

    use super::{CameraSettings, CommonCameraSettings, ExtraMap, Rotation};

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
        address = 0x69
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            bus: Bus::Number(1),
            address: amg88::Address::High,
            frame_rate: amg88::FrameRateValue::Fps10,
            common: CommonCameraSettings::default(),
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grideye_full_bus_num() {
        let source = r#"
        kind = "grideye"
        bus = 3
        address = 0x68
        rotation = 180
        flip_horizontal = true
        flip_vertical = true
        frame_rate = 1
        "#;
        let parsed = toml::from_str(source);
        assert!(parsed.is_ok(), "Unable to parse TOML: {:?}", parsed);
        let parsed: CameraSettings = parsed.unwrap();
        let expected = CameraSettings::GridEye {
            bus: Bus::Number(3),
            address: amg88::Address::Low,
            frame_rate: amg88::FrameRateValue::Fps1,
            common: CommonCameraSettings {
                rotation: Rotation::OneEighty,
                flip_horizontal: true.into(),
                flip_vertical: true.into(),
                extra: ExtraMap::default(),
            },
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
        let expected = CameraSettings::Mlx90640 {
            bus: Bus::Number(1),
            address: 0x33,
            frame_rate: mlx9064x::FrameRate::Eight,
            common: CommonCameraSettings {
                extra: std::iter::once(("path".to_string(), "/foo/bar/baz.bin".into())).collect(),
                ..CommonCameraSettings::default()
            },
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
        let mut extra = ExtraMap::new();
        extra.insert("bus".into(), 1.into());
        extra.insert("address".into(), 0x33.into());
        let expected = CameraSettings::MockCamera {
            path: PathBuf::from("/tmp/qux.bin"),
            frame_rate: 3.0,
            repeat_mode: crate::camera::RepeatMode::default(),
            common: CommonCameraSettings {
                extra,
                ..CommonCameraSettings::default()
            },
        };
        assert_eq!(parsed, expected);
    }
}
