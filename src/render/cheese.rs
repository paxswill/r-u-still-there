// SPDX-License-Identifier: GPL-3.0-or-later
//! This module is a font renderer using the [fontdue] crate. Naming the module 'fontdue' would've
//! been my first choice, but then there'd be a conflict between the module and the crate.
use std::fmt;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use fontdue::layout::{
    CoordinateSystem, HorizontalAlign, Layout, LayoutSettings, TextStyle, VerticalAlign,
};
use fontdue::{Font, FontSettings};

use image::imageops::overlay;
use image::{GenericImage, GrayImage, ImageBuffer, Pixel, Rgb, RgbaImage};
use lru::LruCache;
use tracing::info;

use super::font;
use crate::image_buffer::{BytesImage, ThermalImage};
use crate::render::color;
use crate::temperature::{Temperature, TemperatureUnit};

// Just choosing 50, it felt like a good number.
const CELL_CACHE_SIZE: usize = 50;
pub(crate) struct FontdueRenderer {
    font: Font,
    layout: Arc<Mutex<Layout>>,
    // We can use just the temperature as the key as the text color is dependent on the
    // temperature as well.
    cache: Arc<Mutex<LruCache<(Temperature, u32), BytesImage>>>,
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

    fn render_cell(
        &self,
        temperature: Temperature,
        text_color: color::Color,
        grid_size: u32,
    ) -> BytesImage {
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
        let text = format!("{:.2}", &temperature);
        let style = TextStyle::new(&text, font::FONT_SIZE, 0);
        layout.append(&[&self.font], &style);
        let mut opacity = GrayImage::new(grid_size, grid_size);
        let glyphs = layout.glyphs().clone();
        for glyph in glyphs.iter() {
            let (metrics, bitmap) = self.font.rasterize_config(glyph.key);
            let bitmap = ImageBuffer::from_vec(metrics.width as u32, metrics.height as u32, bitmap)
                .expect("the provided buffer to be large enough");
            overlay(&mut opacity, &bitmap, glyph.x as u32, glyph.y as u32)
        }
        // Combine the provided color with the opacity in `cell`. Also expand to an RGBA image
        // at this point.
        let text_color_pixel: Rgb<_> = text_color.as_array().into();
        let mut cell = RgbaImage::from_pixel(grid_size, grid_size, text_color_pixel.to_rgba());
        cell.pixels_mut()
            .zip(opacity.pixels().map(|pixel| pixel.0[0]))
            .for_each(|(pixel, opacity)| {
                pixel.channels_mut()[3] = opacity;
            });
        ImageBuffer::from_raw(grid_size, grid_size, Bytes::from(cell.into_raw()))
            .expect("the conversion from a Vec ImageBuffer to a Bytes ImageBuffer to work")
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
        temperature_colors: &RgbaImage,
        grid_image: &mut RgbaImage,
    ) -> anyhow::Result<()> {
        let grid_size = grid_size as u32;
        temperatures
            .enumerate_pixels()
            .zip(temperature_colors.pixels())
            // Map the temperature in Celsius to whatever the requested unit is.
            .for_each(|((col, row, temperature_pixel), color_pixel)| {
                let temperature = Temperature::Celsius(temperature_pixel.0[0]).as_unit(&units);
                let cell = {
                    let mut cache = self.cache.lock().unwrap();
                    let mut cached_cell = cache.get(&(temperature, grid_size));
                    if cached_cell.is_none() {
                        info!(?temperature, "cache miss");
                        let text_color = color::Color::from(color_pixel).foreground_color();
                        let mask = self.render_cell(temperature, text_color, grid_size);
                        cache.put((temperature, grid_size), mask);
                        cached_cell = cache.get(&(temperature, grid_size));
                    }
                    cached_cell.unwrap().clone()
                };
                // It'd be nice to use image::imageops::overlay, but it's slow as it has to
                // handle all the cases with alpha channels and such. In our case, we have an
                // opaque background, and a mostly transparent foregound. We can skip all
                // completely transparent foregound pixels and then just blend only those remaining.
                let mut sub_image =
                    grid_image.sub_image(col * grid_size, row * grid_size, grid_size, grid_size);
                cell.enumerate_pixels()
                    .filter(|(_, _, pixel)| pixel.0[3] != 0)
                    .for_each(|(x, y, pixel)| sub_image.blend_pixel(x, y, *pixel));
            });
        Ok(())
    }
}
