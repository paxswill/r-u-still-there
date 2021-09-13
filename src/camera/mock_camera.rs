// SPDX-License-Identifier: GPL-3.0-or-later
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
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
    measurements: Box<dyn Iterator<Item = MeasurementData> + Send + Sync>,
    last_delay: Duration,
}

impl MockCamera {
    pub(crate) fn new_repeating<I>(measurements: I) -> Self
    where
        I: IntoIterator<Item = MeasurementData>,
        <I as IntoIterator>::IntoIter: 'static + Clone + Send + Sync,
    {
        Self {
            frame_rate: 1.0,
            measurements: Box::new(measurements.into_iter().cycle()),
            last_delay: Duration::default(),
        }
    }

    pub(crate) fn new_finite<I>(measurements: I) -> Self
    where
        I: IntoIterator<Item = MeasurementData>,
        <I as IntoIterator>::IntoIter: 'static + Send + Sync,
    {
        Self {
            frame_rate: 1.0,
            measurements: Box::new(measurements.into_iter()),
            last_delay: Duration::default(),
        }
    }
}

impl ThermalCamera for MockCamera {
    fn measure(&mut self) -> anyhow::Result<ThermalMeasurement> {
        let data = self
            .measurements
            .next()
            .ok_or(anyhow!("No more measurements in record"))?;
        // When we loop, the first delay is 0. Instead, just repeat the previous delay. For the
        // very first frame, the delay *will* be zero, as the initial value for `last_delay` is
        // zero.
        if data.delay != Duration::ZERO {
            self.last_delay = data.delay;
        }
        let scaled_delay = self.last_delay.mul_f32(self.frame_rate);
        // Keep the last delay around for if we loop
        trace!(original_delay = ?data.delay, ?scaled_delay, scale = ?self.frame_rate, "Scaled frame rate delay");
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

    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()> {
        self.frame_rate = frame_rate.into();
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_test::{assert_tokens, Token};

    use crate::camera::Measurement;
    use crate::image_buffer::ThermalImage;
    use crate::temperature::Temperature;

    use super::MeasurementData;

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
}
