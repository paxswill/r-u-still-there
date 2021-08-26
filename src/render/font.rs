// SPDX-License-Identifier: GPL-3.0-or-later
use image::GrayImage;

use crate::image_buffer::ThermalImage;
use crate::temperature::TemperatureUnit;

pub(super) const DEJA_VU_SANS: &[u8] = include_bytes!("DejaVuSans-Numbers.ttf");
pub(super) const FONT_SIZE: f32 = 12.0;

pub(crate) trait FontRenderer: std::fmt::Debug {
    /// Render the text for temperatures onto a mask image
    ///
    /// Each temperature in temperatures corresponds to a square `grid_size` pixels wide. The
    /// temperatures in `temperatures` are `f32` values in Celsius, and should be rendered as the
    /// units specified in `units`.
    fn render_text(
        &self,
        grid_size: usize,
        units: TemperatureUnit,
        temperatures: &ThermalImage,
    ) -> anyhow::Result<GrayImage>;
}

/// Create a font renderer based on what has been enabled for this build.
#[cfg(feature = "render_fontdue")]
pub(crate) fn default_renderer() -> Box<dyn FontRenderer + Send + Sync> {
    Box::new(super::cheese::FontdueRenderer::new())
}

/// Create a font renderer based on what has been enabled for this build.
#[cfg(all(not(feature = "render_fontdue"), feature = "render_svg"))]
pub(crate) fn default_renderer() -> Box<dyn FontRenderer + Send + Sync> {
    Box::new(super::svg::SvgRenderer())
}

/// Create a font renderer based on what has been enabled for this build.
#[cfg(all(not(feature = "render_fontdue"), not(feature = "render_svg")))]
pub(crate) fn default_renderer() -> Box<dyn FontRenderer + Send + Sync> {
    panic!("No font rendering backend has been enabled");
}
