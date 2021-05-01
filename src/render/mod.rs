// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

use crate::image_buffer::{BytesImage, ThermalImage};

pub mod color;
pub mod font;

#[cfg(feature = "render_svg")]
mod svg;

#[cfg(feature = "render_svg")]
pub use self::svg::Renderer as SvgRenderer;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Limit {
    /// Set the maximum (or minimum) to the largest (or smallest) value in the current image.
    Dynamic,

    /// Set the maximum (or minimum) to the given value.
    Static(f32),
}

impl Default for Limit {
    fn default() -> Self {
        Self::Dynamic
    }
}

/// Control how the temperature of each pixel is displayed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TemperatureDisplay {
    /// Don't show the temperature.
    Disabled,

    /// Display the temperature in Celsius.
    Celsius,

    /// Display the temperature in fahrenheit.
    Fahrenheit,
}

impl Default for TemperatureDisplay {
    fn default() -> Self {
        Self::Disabled
    }
}

pub trait Renderer: Default {
    fn new(
        scale_min: Limit,
        scale_max: Limit,
        display_temperature: TemperatureDisplay,
        grid_size: usize,
        gradient: colorous::Gradient,
    ) -> Self;

    fn scale_min(&self) -> Limit;

    fn scale_max(&self) -> Limit;

    fn display_temperature(&self) -> TemperatureDisplay;

    fn grid_size(&self) -> usize;

    fn set_grid_size(&mut self, grid_size: usize);

    fn gradient(&self) -> colorous::Gradient;

    fn set_gradient(&mut self, gradient: colorous::Gradient);

    fn render_buffer(&self, image: &ThermalImage) -> BytesImage;

    fn color_map(&self, image: &ThermalImage) -> Box<dyn Fn(&f32) -> color::Color> {
        let scale_min = match self.scale_min() {
            Limit::Static(n) => n,
            Limit::Dynamic => {
                *(image
                    .iter()
                    .filter(|n| !n.is_nan())
                    .min_by(|l, r| l.partial_cmp(&r).unwrap())
                    .unwrap())
            }
        };
        let scale_max = match self.scale_max() {
            Limit::Static(n) => n,
            Limit::Dynamic => {
                *(image
                    .iter()
                    .filter(|n| !n.is_nan())
                    .max_by(|l, r| l.partial_cmp(&r).unwrap())
                    .unwrap())
            }
        };
        let scale_range = scale_max - scale_min;
        // Clone the gradient so that it can be owned by the closure
        let gradient = self.gradient();
        Box::new(move |temperature: &f32| -> color::Color {
            color::Color::from(
                gradient.eval_continuous(((temperature - scale_min) / scale_range) as f64),
            )
        })
    }
}
