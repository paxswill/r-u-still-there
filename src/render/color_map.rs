// SPDX-License-Identifier: GPL-3.0-or-later
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::FutureExt;
use image::{Pixel, RgbaImage};
use tokio::task::spawn_blocking;
use tracing::{instrument, trace};

use crate::camera::Measurement;
use crate::image_buffer::ThermalImage;
use crate::temperature::TemperatureUnit;
use crate::util::flatten_join_result;
use crate::util::{Filter, MovingAverage};

use super::settings::{self, RenderSettings};

const DYNAMIC_AVERAGE_NUM: usize = 10;

/// The limits as used by color mappers.
#[derive(Clone, Debug, PartialEq)]
enum Limit {
    /// Set the maximum (or minimum) to the largest (or smallest) value in the current image.
    Dynamic(MovingAverage<f32, DYNAMIC_AVERAGE_NUM>),

    /// Set the maximum (or minimum) to the given value.
    Static(f32),
}

impl Limit {
    fn update<F>(&mut self, image: &ThermalImage, reducer: F) -> f32
    where
        F: Fn(f32, f32) -> f32,
    {
        match self {
            Limit::Dynamic(avg) => {
                let new_value = image
                    .iter()
                    .copied()
                    .reduce(reducer)
                    .expect("An image should have some values");
                avg.update(new_value)
            }
            Limit::Static(static_value) => *static_value,
        }
    }

    fn current_value(&self) -> f32 {
        match self {
            Limit::Dynamic(avg) => avg
                .current_value()
                .expect("A Dynamic Limit needs at least one input before it has a value"),
            Limit::Static(static_value) => *static_value,
        }
    }
}

impl Default for Limit {
    fn default() -> Self {
        Self::Dynamic(MovingAverage::new())
    }
}

impl From<settings::Limit> for Limit {
    fn from(settings_limit: settings::Limit) -> Self {
        match settings_limit {
            settings::Limit::Dynamic => Self::Dynamic(MovingAverage::new()),
            settings::Limit::Static(temperature) => {
                Self::Static(temperature.in_unit(&TemperatureUnit::Celsius))
            }
        }
    }
}

#[async_trait]
pub(crate) trait ColorMapper: std::fmt::Debug {
    async fn render(&self, measurement: Measurement) -> anyhow::Result<RgbaImage>;
}

#[derive(Clone, Debug)]
pub(crate) struct ImageColorMap {
    scale_min: Arc<Mutex<Limit>>,
    scale_max: Arc<Mutex<Limit>>,
    gradient: colorous::Gradient,
}

/// A color mapper using the [`image`] crate.
impl ImageColorMap {
    pub(crate) fn new(
        scale_min: settings::Limit,
        scale_max: settings::Limit,
        gradient: colorous::Gradient,
    ) -> Self {
        Self {
            scale_min: Arc::new(Mutex::new(scale_min.into())),
            scale_max: Arc::new(Mutex::new(scale_max.into())),
            gradient,
        }
    }

    /// The smallest difference between the upper and lower limits when dynamic limits are in use.
    ///
    /// If only one limit is dynamic, it will be raised or lowered the satisfy this constraint. If
    /// both limits are dynamic, the upper limit will be raised to satisfy this range.
    const MINIMUM_DYNAMIC_RANGE: f32 = 5.0;
}

impl Default for ImageColorMap {
    fn default() -> Self {
        Self::new(
            settings::Limit::default(),
            settings::Limit::default(),
            colorous::TURBO,
        )
    }
}

impl<'a> From<&'a RenderSettings> for ImageColorMap {
    fn from(settings: &'a RenderSettings) -> Self {
        Self::new(
            settings.lower_limit,
            settings.upper_limit,
            settings.colors.into(),
        )
    }
}

#[async_trait]
impl ColorMapper for ImageColorMap {
    #[instrument(level = "debug", skip(measurement))]
    async fn render(&self, measurement: Measurement) -> anyhow::Result<RgbaImage> {
        let scale_min = Arc::clone(&self.scale_min);
        let scale_max = Arc::clone(&self.scale_max);
        let gradient = self.gradient;
        spawn_blocking(move || {
            // Map the thermal image to an actual RGB image. We're converting to RGBA at the same time
            // as that's what resvg wants.
            let source_width = measurement.image.width();
            let source_height = measurement.image.height();
            // Find the range of the thermal image if there are any dynamic limits
            let mut scale_min = scale_min.lock().unwrap();
            scale_min.update(&measurement.image, f32::min);
            let mut scale_max = scale_max.lock().unwrap();
            scale_max.update(&measurement.image, f32::max);
            // Now to ensure the minimum range is maintained
            let (new_min, new_max) = match (scale_min.deref(), scale_max.deref()) {
                // Same operation if the lower limit is static or dynamic and the upper is dynamic
                (lower_limit, Limit::Dynamic(upper_limit_avg)) => {
                    let upper_limit = upper_limit_avg.current_value().unwrap();
                    let lower_limit = lower_limit.current_value();
                    if upper_limit - lower_limit < Self::MINIMUM_DYNAMIC_RANGE {
                        (lower_limit, lower_limit + Self::MINIMUM_DYNAMIC_RANGE)
                    } else {
                        (lower_limit, upper_limit)
                    }
                }
                (Limit::Dynamic(lower_limit_avg), Limit::Static(upper_limit)) => {
                    let lower_limit = lower_limit_avg.current_value().unwrap();
                    if upper_limit - lower_limit < Self::MINIMUM_DYNAMIC_RANGE {
                        (upper_limit - Self::MINIMUM_DYNAMIC_RANGE, *upper_limit)
                    } else {
                        (lower_limit, *upper_limit)
                    }
                }
                (lower_limit, upper_limit) => {
                    (upper_limit.current_value(), lower_limit.current_value())
                }
            };
            let scale_range = new_max - new_min;
            // Scale the input temperatures to a value 0-1.0
            let scaled_values = measurement
                .image
                .iter()
                .map(|temperature| (temperature - new_min) / scale_range);
            // Use the colorous gradient to map the scaled values into colors
            let mut temperature_colors = image::RgbaImage::new(source_width, source_height);
            for (source, dest) in scaled_values.zip(temperature_colors.pixels_mut()) {
                let gradient_color =
                    image::Rgb::from(gradient.eval_continuous(source as f64).as_array());
                *dest = gradient_color.to_rgba();
            }
            trace!("mapped temperatures to colors");
            let full_width = temperature_colors.width();
            let full_height = temperature_colors.height();
            trace!(
                source_width,
                source_height,
                enlarged_width = full_width,
                enlarged_height = full_height,
                "enlarged source image"
            );
            anyhow::Result::<RgbaImage>::Ok(temperature_colors)
        })
        .map(flatten_join_result)
        .await
    }
}
