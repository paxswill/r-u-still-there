// SPDX-License-Identifier: GPL-3.0-or-later
use std::num::NonZeroU8;

use serde::Deserialize;
use serde_repr::Deserialize_repr;

use super::{Bus, I2cSettings};

// This enum is purely used to restrict the acceptable values for rotation
#[derive(Clone, Copy, Deserialize_repr, PartialEq, Debug)]
#[repr(u16)]
pub(crate) enum Rotation {
    Zero = 0,
    Ninety = 90,
    OneEighty = 180,
    TwoSeventy = 270,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum CameraKind {
    GridEye(I2cSettings),
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CameraSettings {
    #[serde(flatten)]
    pub(crate) kind: CameraKind,

    #[serde(default)]
    pub(crate) rotation: Rotation,

    #[serde(default)]
    pub(crate) flip_horizontal: bool,

    #[serde(default)]
    pub(crate) flip_vertical: bool,

    #[serde(default)]
    frame_rate: Option<NonZeroU8>,
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
        }
    }

    pub(crate) fn set_bus(&mut self, new_bus: Bus) {
        match self {
            CameraKind::GridEye(i2c) => i2c.bus = new_bus,
        }
    }

    pub(crate) fn set_address(&mut self, new_address: u8) {
        match self {
            CameraKind::GridEye(i2c) => i2c.address = new_address,
        }
    }
}

impl CameraSettings {
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
        };
        assert_eq!(parsed, expected);
    }
}
