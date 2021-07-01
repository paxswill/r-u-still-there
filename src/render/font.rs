// SPDX-License-Identifier: GPL-3.0-or-later
use image::RgbaImage;

use crate::image_buffer::ThermalImage;
use crate::temperature::TemperatureUnit;

pub(super) const DEJA_VU_SANS: &[u8] = include_bytes!("DejaVuSans-Numbers.ttf");
pub(super) const FONT_SIZE: f32 = 12.0;

pub(crate) trait FontRenderer: std::fmt::Debug {
    /// Render the text for temperatures from `temperatures` on to `grid_image`, in the temperature
    /// units specified. The colors in `temperature_colors` can be used to choose a text color with
    /// for better contrast. Each temperature "pixel" in `temperatures` corresponds to a pixel in
    /// `temperature_colors`. Each temperature is represented by a `grid_size` pixel square in
    /// `grid_image`.
    fn render_text(
        &self,
        grid_size: usize,
        units: TemperatureUnit,
        temperatures: &ThermalImage,
        temperature_colors: &RgbaImage,
        grid_image: &mut RgbaImage,
    ) -> anyhow::Result<()>;
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