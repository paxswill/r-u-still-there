// SPDX-License-Identifier: GPL-3.0-or-later
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use serde::de::{self, Deserialize, Deserializer, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeStruct};
use tracing::trace;

use crate::image_buffer::ThermalImage;
use crate::temperature::TaggedTemperature;

use super::measurement::Measurement;
use super::thermal_camera::{Measurement as ThermalMeasurement, ThermalCamera, YAxisDirection};

/// A wrapper around [`Measurement`] data so that it can be serialized to a file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MeasurementData {
    measurement: Measurement,
    delay: Duration,
}

impl MeasurementData {
    pub(crate) fn new(measurement: Measurement, delay: Duration) -> Self {
        Self { measurement, delay }
    }
}

impl From<MeasurementData> for Measurement {
    fn from(data: MeasurementData) -> Self {
        data.measurement
    }
}

impl Serialize for MeasurementData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut flattened = serializer.serialize_struct("MeasurementData", 5)?;
        // Serialize the dimensions before the values, so you know how many values there are.
        flattened.serialize_field("width", &self.measurement.image.width())?;
        flattened.serialize_field("height", &self.measurement.image.height())?;
        flattened.serialize_field("values", self.measurement.image.as_raw())?;
        flattened.serialize_field("temperature", &self.measurement.temperature)?;
        flattened.serialize_field("delay", &self.delay)?;
        flattened.end()
    }
}

impl<'de> Deserialize<'de> for MeasurementData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field<'a> {
            Width,
            Height,
            Values,
            Temperature,
            Delay,
            Unknown(&'a str),
        }

        struct DataVisitor;

        impl<'de> Visitor<'de> for DataVisitor {
            type Value = MeasurementData;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("struct MeasurementData")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let width: u32 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let height: u32 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let pixels: Vec<f32> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let temperature: TaggedTemperature = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                let delay = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                let image: ThermalImage =
                    ThermalImage::from_vec(width as u32, height as u32, pixels)
                        .ok_or_else(|| de::Error::custom("Image buffer was not large enough"))?;
                Ok(MeasurementData {
                    measurement: Measurement {
                        image: Arc::new(image),
                        temperature: temperature.into(),
                    },
                    delay,
                })
            }

            fn visit_map<V>(self, mut map: V) -> Result<MeasurementData, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut width = None;
                let mut height = None;
                let mut values = None;
                let mut temperature = None;
                let mut delay = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Width => {
                            if width.is_some() {
                                return Err(de::Error::duplicate_field("width"));
                            }
                            width = Some(map.next_value()?);
                        }
                        Field::Height => {
                            if height.is_some() {
                                return Err(de::Error::duplicate_field("height"));
                            }
                            height = Some(map.next_value()?);
                        }
                        Field::Values => {
                            if values.is_some() {
                                return Err(de::Error::duplicate_field("values"));
                            }
                            values = Some(map.next_value()?);
                        }
                        Field::Temperature => {
                            if temperature.is_some() {
                                return Err(de::Error::duplicate_field("temperature"));
                            }
                            temperature = Some(map.next_value()?);
                        }
                        Field::Delay => {
                            if delay.is_some() {
                                return Err(de::Error::duplicate_field("delay"));
                            }
                            delay = Some(map.next_value()?);
                        }
                        Field::Unknown(_) => {}
                    }
                }
                let width: u32 = width.ok_or_else(|| de::Error::missing_field("width"))?;
                let height: u32 = height.ok_or_else(|| de::Error::missing_field("height"))?;
                let image_data: Vec<f32> =
                    values.ok_or_else(|| de::Error::missing_field("values"))?;
                if (width * height) as usize != image_data.len() {
                    return Err(de::Error::invalid_length(
                        image_data.len(),
                        &"the values list should match the dimensions",
                    ));
                }
                let image: ThermalImage =
                    ThermalImage::from_vec(width as u32, height as u32, image_data)
                        .ok_or_else(|| de::Error::custom("Image buffer was not large enough"))?;
                let temperature =
                    temperature.ok_or_else(|| de::Error::missing_field("temperature"))?;
                let delay = delay.ok_or_else(|| de::Error::missing_field("delay"))?;
                Ok(MeasurementData {
                    measurement: Measurement {
                        image: Arc::new(image),
                        temperature,
                    },
                    delay,
                })
            }
        }

        const FIELDS: &'static [&'static str] =
            &["values", "width", "height", "temperature", "delay"];
        deserializer.deserialize_struct("MeasurementData", FIELDS, DataVisitor)
    }
}

pub(crate) struct MockCamera {
    frame_rate: f32,
    measurements: Vec<MeasurementData>,
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
    pub(crate) fn new(measurements: Vec<MeasurementData>, repeat: RepeatMode) -> Self {
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
    fn measure(&mut self) -> anyhow::Result<ThermalMeasurement> {
        let index = self
            .index
            .next()
            .ok_or(anyhow!("No more measurements in record"))?;
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
        Ok(ThermalMeasurement {
            image: image,
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
    use serde_test::{assert_tokens, Token};

    use crate::camera::Measurement;
    use crate::image_buffer::ThermalImage;
    use crate::temperature::Temperature;

    use super::super::thermal_camera::{Measurement as ThermalMeasurement, ThermalCamera};
    use super::{MeasurementData, MockCamera, RepeatMode};

    #[test]
    fn measurement_data_format() {
        // Choosing prime numbers to be annoying
        const HEIGHT: u32 = 11;
        const WIDTH: u32 = 17;
        const NUM_PIXELS: usize = (HEIGHT * WIDTH) as usize;
        let empty_image = ThermalImage::new(WIDTH, HEIGHT);
        let measurement = Measurement {
            image: Arc::new(empty_image),
            temperature: Temperature::Celsius(28.0),
        };
        let delay = Duration::from_millis(125);
        let record = MeasurementData::new(measurement, delay);
        let mut tokens = vec![
            // Start MeasurementData
            Token::Struct {
                name: "MeasurementData",
                len: 5,
            },
            // width (u32)
            Token::Str("width"),
            Token::U32(WIDTH),
            // height (u32)
            Token::Str("height"),
            Token::U32(HEIGHT),
            Token::Str("values"),
            // Start values (Vec<f32>)
            Token::Seq {
                len: Some(NUM_PIXELS),
            },
        ];
        // A whole bunch of zeros for pixels
        tokens.extend(std::iter::repeat(Token::F32(0.0)).take(NUM_PIXELS));
        // Everything after the pixels
        tokens.extend(&[
            // End Vec<f32>
            Token::SeqEnd,
            // temperature (TaggedTemperature)
            Token::Str("temperature"),
            Token::NewtypeVariant {
                name: "TaggedTemperature",
                variant: "celsius",
            },
            Token::F32(28.0),
            // delay (Duration)
            Token::Str("delay"),
            // Not explicitly documented, but Duration is serialized as { secs: u64, nanos: u32 }
            Token::Struct {
                name: "Duration",
                len: 2,
            },
            Token::Str("secs"),
            Token::U64(delay.as_secs()),
            Token::Str("nanos"),
            Token::U32(delay.subsec_nanos()),
            // End Duration
            Token::StructEnd,
            // End MeasurementData
            Token::StructEnd,
        ]);
        assert_tokens(&record, &tokens[..]);
    }

    const START_IMAGE_TEMP: f32 = 20.0;

    const START_AMBIENT_TEMP: f32 = 30.0;

    const NUM_TINY_MEASUREMENTS: usize = 10;

    fn tiny_measurements() -> Vec<MeasurementData> {
        let delay = Duration::from_millis(25);
        (0..NUM_TINY_MEASUREMENTS)
            .map(|offset| {
                let offset = offset as f32;
                let temperature = Temperature::Celsius(START_AMBIENT_TEMP + offset);
                let image = ThermalImage::from_pixel(1, 1, [START_IMAGE_TEMP + offset].into());
                MeasurementData::new(
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
        let measurements: Vec<ThermalMeasurement> = std::iter::from_fn(move || cam.measure().ok())
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
