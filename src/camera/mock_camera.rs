// SPDX-License-Identifier: GPL-3.0-or-later
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use serde::de::{Deserialize, IntoDeserializer};
use tracing::trace;

use crate::recorded_data::RecordedData;

use super::thermal_camera::{CameraSample, ThermalCamera, YAxisDirection};

pub(crate) struct MockCamera {
    frame_rate: f32,
    measurements: Vec<RecordedData>,
    index: Box<dyn Iterator<Item = usize> + Send + Sync>,
    last_delay: Duration,
}

/// Controls how measurements are repeated by [`MockCamera`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RepeatMode {
    /// Don't repeat.
    ///
    /// Once the end of the measurements has been reached, an error is returned.
    None,

    /// Loop over the measurements.
    ///
    /// Once the end of the measurements has been reached, the list is restarted from the
    /// beginning, reusing the last delay for the first frame. This is the default mode.
    Loop,

    /// Alternate between forward and reverse playback.
    ///
    /// Once the end of the measurements has been reached, playback continues backwards. Once the
    /// beginning has been reached, playback continues forwards. The measurements at either end of
    /// the list of data are *not* repeated.
    Bounce,
}

impl RepeatMode {
    pub(crate) const KINDS: &'static [&'static str] = &["none", "loop", "bounce"];
}

impl Default for RepeatMode {
    fn default() -> Self {
        Self::Loop
    }
}

impl FromStr for RepeatMode {
    type Err = serde::de::value::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RepeatMode::deserialize(s.into_deserializer())
    }
}

impl fmt::Display for RepeatMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            RepeatMode::None => "none",
            RepeatMode::Loop => "loop",
            RepeatMode::Bounce => "bounce",
        };
        write!(f, "{}", s)
    }
}

impl MockCamera {
    pub(crate) fn new(measurements: Vec<RecordedData>, repeat: RepeatMode) -> Self {
        let num_measurements = measurements.len();
        let index: Box<dyn Iterator<Item = usize> + Send + Sync> = match repeat {
            RepeatMode::None => Box::new(0..num_measurements),
            RepeatMode::Loop => Box::new((0..num_measurements).cycle()),
            RepeatMode::Bounce => {
                let forwards = 0..num_measurements;
                let backwards = (1..(num_measurements - 1)).rev();
                Box::new(forwards.chain(backwards).cycle())
            }
        };
        Self {
            frame_rate: 1.0,
            measurements,
            index,
            last_delay: Duration::ZERO,
        }
    }
}

impl ThermalCamera for MockCamera {
    fn sample(&mut self) -> anyhow::Result<CameraSample> {
        let index = self
            .index
            .next()
            .ok_or_else(|| anyhow!("No more measurements in record"))?;
        let data = self.measurements[index].clone();
        // When we loop, the first delay is 0. Instead, just repeat the previous delay. For the
        // very first frame, the delay *will* be zero, as the initial value for `last_delay` is
        // zero.
        if data.delay != Duration::ZERO {
            self.last_delay = data.delay;
        }
        let scaled_delay = self.last_delay.mul_f32(self.frame_rate);
        // Keep the last delay around for if we loop
        trace!(
            original_delay = ?data.delay,
            ?scaled_delay,
            scale = ?self.frame_rate, "Scaled frame rate delay"
        );
        let image = Arc::try_unwrap(data.measurement.image).unwrap_or_else(|arc| {
            // If we can't take ownership of the Arc, clone the inner data instead.
            arc.as_ref().clone()
        });
        Ok(CameraSample {
            image,
            y_direction: YAxisDirection::Down,
            temperature: data.measurement.temperature,
            frame_delay: scaled_delay,
        })
    }

    fn set_frame_rate(&mut self, frame_rate: f32) -> anyhow::Result<()> {
        self.frame_rate = frame_rate;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use std::time::Duration;

    use image::Pixel;

    use crate::camera::Measurement;
    use crate::image_buffer::ThermalImage;
    use crate::recorded_data::RecordedData;
    use crate::temperature::Temperature;

    use super::super::thermal_camera::{CameraSample, ThermalCamera};
    use super::{MockCamera, RepeatMode};

    const START_IMAGE_TEMP: f32 = 20.0;

    const START_AMBIENT_TEMP: f32 = 30.0;

    const NUM_TINY_MEASUREMENTS: usize = 10;

    fn tiny_measurements() -> Vec<RecordedData> {
        let delay = Duration::from_millis(25);
        (0..NUM_TINY_MEASUREMENTS)
            .map(|offset| {
                let offset = offset as f32;
                let temperature = Temperature::Celsius(START_AMBIENT_TEMP + offset);
                let image = ThermalImage::from_pixel(1, 1, [START_IMAGE_TEMP + offset].into());
                RecordedData::new(
                    Measurement {
                        image: Arc::new(image),
                        temperature,
                    },
                    delay,
                )
            })
            .collect()
    }

    fn assert_measurements(repeat_mode: RepeatMode, image_temps: &[f32], ambient_temps: &[f32]) {
        assert_eq!(
            image_temps.len(),
            ambient_temps.len(),
            "image_temps and ambient_temps must be the same length"
        );
        let mut cam = MockCamera::new(tiny_measurements(), repeat_mode);
        let measurements: Vec<CameraSample> = std::iter::from_fn(move || cam.sample().ok())
            .fuse()
            .take(30)
            .collect();
        let expected_length = image_temps.len();
        assert_eq!(
            measurements.len(),
            expected_length,
            "Unexpected number of measurements"
        );
        let actual_image_temps: Vec<f32> = measurements
            .iter()
            .map(|m| m.image[(0, 0)].channels()[0])
            .collect();
        let actual_ambient_temps: Vec<f32> = measurements
            .iter()
            .map(|m| m.temperature.in_celsius())
            .collect();
        assert_eq!(
            &actual_image_temps[..],
            image_temps,
            "image tmperatures do not match"
        );
        assert_eq!(
            &actual_ambient_temps[..],
            ambient_temps,
            "ambient tmperatures do not match"
        );
    }

    #[test]
    fn repeat_none() {
        let expected_image = [20.0, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 29.0];
        let expected_ambient = [30.0, 31.0, 32.0, 33.0, 34.0, 35.0, 36.0, 37.0, 38.0, 39.0];
        assert_measurements(RepeatMode::None, &expected_image[..], &expected_ambient[..])
    }

    #[test]
    fn repeat_loop() {
        let expected_image = [
            20.0, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 29.0, 20.0, 21.0, 22.0, 23.0,
            24.0, 25.0, 26.0, 27.0, 28.0, 29.0, 20.0, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0,
            28.0, 29.0,
        ];
        let expected_ambient = [
            30.0, 31.0, 32.0, 33.0, 34.0, 35.0, 36.0, 37.0, 38.0, 39.0, 30.0, 31.0, 32.0, 33.0,
            34.0, 35.0, 36.0, 37.0, 38.0, 39.0, 30.0, 31.0, 32.0, 33.0, 34.0, 35.0, 36.0, 37.0,
            38.0, 39.0,
        ];
        assert_measurements(RepeatMode::Loop, &expected_image[..], &expected_ambient[..])
    }

    #[test]
    fn repeat_bounce() {
        // NOTE: bounce does *not* repeat each end of the loop
        let expected_image = [
            20.0, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 29.0, 28.0, 27.0, 26.0, 25.0,
            24.0, 23.0, 22.0, 21.0, 20.0, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 29.0,
            28.0, 27.0,
        ];
        let expected_ambient = [
            30.0, 31.0, 32.0, 33.0, 34.0, 35.0, 36.0, 37.0, 38.0, 39.0, 38.0, 37.0, 36.0, 35.0,
            34.0, 33.0, 32.0, 31.0, 30.0, 31.0, 32.0, 33.0, 34.0, 35.0, 36.0, 37.0, 38.0, 39.0,
            38.0, 37.0,
        ];
        assert_measurements(
            RepeatMode::Bounce,
            &expected_image[..],
            &expected_ambient[..],
        )
    }
}
