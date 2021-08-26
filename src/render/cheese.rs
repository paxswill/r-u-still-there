// SPDX-License-Identifier: GPL-3.0-or-later
//! This module is a font renderer using the [fontdue] crate. Naming the module 'fontdue' would've
//! been my first choice, but then there'd be a conflict between the module and the crate.
use std::fmt;
use std::sync::{Arc, Mutex};

use fontdue::layout::{
    CoordinateSystem, HorizontalAlign, Layout, LayoutSettings, TextStyle, VerticalAlign,
};
use fontdue::{Font, FontSettings};

use image::error::ImageResult;
use image::imageops::overlay;
use image::{GenericImage, GrayImage, ImageBuffer};
use lru::LruCache;
use tracing::trace;

use super::font;
use crate::image_buffer::ThermalImage;
use crate::temperature::{Temperature, TemperatureUnit};

// Just choosing 50, it felt like a good number.
const CELL_CACHE_SIZE: usize = 50;
pub(crate) struct FontdueRenderer {
    font: Font,
    layout: Arc<Mutex<Layout>>,
    // We can use just the temperature as the key as the text color is dependent on the
    // temperature as well.
    cache: Arc<Mutex<LruCache<(Temperature, u32), GrayImage>>>,
}

#[cfg(feature = "render_fontdue")]
impl FontdueRenderer {
    pub(crate) fn new() -> Self {
        let font = Font::from_bytes(font::DEJA_VU_SANS, FontSettings::default()).unwrap();
        Self {
            font,
            layout: Arc::new(Mutex::new(Layout::new(CoordinateSystem::PositiveYDown))),
            cache: Arc::new(Mutex::new(LruCache::new(CELL_CACHE_SIZE))),
        }
    }

    fn render_cell(&self, temperature: Temperature, grid_size: u32) -> GrayImage {
        // Reset the fontdue context to a known default
        let mut layout = self.layout.lock().unwrap();
        layout.reset(&LayoutSettings {
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
        layout.append(&[&self.font], &style);
        // Transfer the rasterized glyphs from fontdue onto an image mask. The mask is just the
        // opacity for each pixel in a cell.
        let mut mask = GrayImage::new(grid_size, grid_size);
        let glyphs = layout.glyphs().clone();
        for glyph in glyphs.iter() {
            let (metrics, bitmap) = self.font.rasterize_config(glyph.key);
            let bitmap = ImageBuffer::from_vec(metrics.width as u32, metrics.height as u32, bitmap)
                .expect("the provided buffer to be large enough");
            overlay(&mut mask, &bitmap, glyph.x as u32, glyph.y as u32)
        }
        mask
    }
}

impl fmt::Debug for FontdueRenderer {
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

impl font::FontRenderer for FontdueRenderer {
    fn render_text(
        &self,
        grid_size: usize,
        units: TemperatureUnit,
        temperatures: &ThermalImage,
    ) -> anyhow::Result<GrayImage> {
        let grid_size = grid_size as u32;
        let mut full_mask = GrayImage::new(
            grid_size * temperatures.width(),
            grid_size * temperatures.height(),
        );
        temperatures
            .enumerate_pixels()
            // Map the temperature in Celsius to whatever the requested unit is.
            .map(|(col, row, temperature_pixel)| {
                let temperature = Temperature::Celsius(temperature_pixel.0[0]).as_unit(&units);
                let mut cache = self.cache.lock().unwrap();
                let mut cached_cell = cache.get(&(temperature, grid_size));
                if cached_cell.is_none() {
                    trace!(?temperature, "cache miss");
                    let mask = self.render_cell(temperature, grid_size);
                    cache.put((temperature, grid_size), mask);
                    cached_cell = cache.get(&(temperature, grid_size));
                }
                let cell = cached_cell.unwrap();
                full_mask.copy_from(cell, col * grid_size, row * grid_size)
            })
            .collect::<ImageResult<()>>()?;
        Ok(full_mask)
    }
}
