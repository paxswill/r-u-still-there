// SPDX-License-Identifier: GPL-3.0-or-later
//! This module is a font renderer using the [fontdue] crate. Naming the module 'fontdue' would've
//! been my first choice, but then there'd be a conflict between the module and the crate.
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fontdue::layout::{
    CoordinateSystem, HorizontalAlign, Layout, LayoutSettings, TextStyle, VerticalAlign,
};
use fontdue::{Font, FontSettings};
use futures::FutureExt;
use image::error::ImageResult;
use image::imageops::overlay;
use image::{GenericImage, GrayImage, ImageBuffer};
use lru::LruCache;
use tokio::task::spawn_blocking;
use tracing::trace;

use super::font;
use crate::camera::Measurement;
use crate::temperature::{Temperature, TemperatureUnit};
use crate::util::flatten_join_result;

// Just choosing 50, it felt like a good number.
const CELL_CACHE_SIZE: usize = 50;

struct InnerRenderer {
    font: Font,
    layout: Layout,
    // We can use just the temperature as the key as the text color is dependent on the
    // temperature as well.
    cache: LruCache<(Temperature, u32), GrayImage>,
}

impl InnerRenderer {
    fn new() -> Self {
        let font = Font::from_bytes(font::DEJA_VU_SANS, FontSettings::default()).unwrap();
        Self {
            font,
            layout: Layout::new(CoordinateSystem::PositiveYDown),
            cache: LruCache::new(CELL_CACHE_SIZE),
        }
    }

    fn render_cell(&mut self, temperature: Temperature, grid_size: u32) -> GrayImage {
        // Reset the fontdue context to a known default
        self.layout.reset(&LayoutSettings {
            x: 0.0,
            y: 0.0,
            max_height: Some(grid_size as f32),
            max_width: Some(grid_size as f32),
            horizontal_align: HorizontalAlign::Center,
            vertical_align: VerticalAlign::Middle,
            ..LayoutSettings::default()
        });
        // Add the text we're rendering to the fontdue context
        let text = format!("{:.2}", &temperature);
        let style = TextStyle::new(&text, font::FONT_SIZE, 0);
        self.layout.append(&[&self.font], &style);
        // Transfer the rasterized glyphs from fontdue onto an image mask. The mask is just the
        // opacity for each pixel in a cell.
        let mut mask = GrayImage::new(grid_size, grid_size);
        let glyphs = self.layout.glyphs().clone();
        for glyph in glyphs.iter() {
            let (metrics, bitmap) = self.font.rasterize_config(glyph.key);
            let bitmap = ImageBuffer::from_vec(metrics.width as u32, metrics.height as u32, bitmap)
                .expect("the provided buffer to be large enough");
            overlay(&mut mask, &bitmap, glyph.x as u32, glyph.y as u32)
        }
        mask
    }
}

impl fmt::Debug for InnerRenderer {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // fontdue::layout::Layout doesn't implement Debug, so instead I'm just putting a dummy
        // blob in there.
        fmt.debug_struct("FontdueRenderer")
            .field("font", &self.font)
            .field("layout", &"Arc<Mutex<Layout{{ opaque }}>>")
            .field("cache", &self.cache)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct FontdueRenderer {
    inner: Arc<Mutex<InnerRenderer>>,
}

#[cfg(feature = "render_fontdue")]
impl FontdueRenderer {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(InnerRenderer::new())),
        }
    }
}

#[async_trait]
impl font::FontRenderer for FontdueRenderer {
    async fn render(
        &self,
        grid_size: usize,
        units: TemperatureUnit,
        measurement: Measurement,
    ) -> anyhow::Result<GrayImage> {
        let inner = Arc::clone(&self.inner);
        spawn_blocking(move || {
            let temperatures = measurement.image;
            let grid_size = grid_size as u32;
            let mut full_mask = GrayImage::new(
                grid_size * temperatures.width(),
                grid_size * temperatures.height(),
            );
            let mut inner = inner.lock().unwrap();
            temperatures
                .enumerate_pixels()
                // Map the temperature in Celsius to whatever the requested unit is.
                .map(|(col, row, temperature_pixel)| {
                    let temperature = Temperature::Celsius(temperature_pixel.0[0]).as_unit(&units);
                    let mut cached_cell = inner.cache.get(&(temperature, grid_size));
                    if cached_cell.is_none() {
                        trace!(?temperature, "cache miss");
                        let mask = inner.render_cell(temperature, grid_size);
                        inner.cache.put((temperature, grid_size), mask);
                        cached_cell = inner.cache.get(&(temperature, grid_size));
                    }
                    let cell = cached_cell.unwrap();
                    full_mask.copy_from(cell, col * grid_size, row * grid_size)
                })
                .collect::<ImageResult<()>>()?;
            anyhow::Result::<GrayImage>::Ok(full_mask)
        })
        .map(flatten_join_result)
        .await
    }
}
