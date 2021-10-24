// SPDX-License-Identifier: GPL-3.0-or-later
use async_trait::async_trait;
use image::GrayImage;

use crate::camera::Measurement;
use crate::temperature::TemperatureUnit;

pub(super) const DEJA_VU_SANS: &[u8] = include_bytes!("DejaVuSans-Numbers.ttf");
pub(super) const FONT_SIZE: f32 = 12.0;

#[async_trait]
pub(crate) trait FontRenderer: std::fmt::Debug {
    /// Render the text for temperatures onto a mask image
    ///
    /// Each temperature in temperatures corresponds to a square `grid_size` pixels wide. The
    /// temperatures in `temperatures` are `f32` values in Celsius, and should be rendered as the
    /// units specified in `units`.
    async fn render(
        &self,
        grid_size: usize,
        units: TemperatureUnit,
        measurement: Measurement,
    ) -> anyhow::Result<GrayImage>;
}

/// Create a font renderer based on what has been enabled for this build.
pub(crate) fn default_renderer() -> Box<dyn FontRenderer + Send + Sync> {
    Box::new(super::cheese::FontdueRenderer::new())
}
