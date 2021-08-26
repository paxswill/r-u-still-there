// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::anyhow;
use bytes::Bytes;
use image::{Pixel, Rgba};

use crate::camera::Measurement;
use crate::image_buffer::BytesImage;

use super::background::{BackgroundRenderer, ImageBackground};
use super::color::Color;
use super::font::{default_renderer, FontRenderer};
use super::settings::RenderSettings;
use super::TemperatureDisplay;

#[derive(Debug)]
pub(crate) struct ImageLayers {
    background_renderer: ImageBackground,
    font_renderer: Option<Box<dyn FontRenderer + Send + Sync>>,
    display: TemperatureDisplay,
    grid_size: usize,
    display_temperature: TemperatureDisplay,
}

impl ImageLayers {
    pub(crate) fn render(&mut self, measurement: Measurement) -> anyhow::Result<BytesImage> {
        let mut background = self.background_renderer.render(&measurement);
        let font_layer = match self.display_temperature {
            TemperatureDisplay::Disabled => None,
            TemperatureDisplay::Absolute(unit) => {
                let font_mask = self
                    .font_renderer
                    .as_mut()
                    .ok_or(anyhow!(
                        "Font renderer not created for displayed temperature units"
                    ))?
                    .render_text(self.grid_size, unit, &measurement.image)?;
                Some(font_mask)
            }
        };
        // Flatten layers
        if let Some(font_mask) = font_layer {
            background
                .pixels_mut()
                .zip(font_mask.iter())
                // We only need to modify pixels that have some font data in them, and opacity is
                // the easy filter for that.
                .filter(|(_, opacity)| **opacity != 0)
                .for_each(|(background, text_mask)| {
                    let not_mut: &Rgba<u8> = background;
                    let mut text_color: Rgba<u8> = Color::from(not_mut).foreground_color().into();
                    text_color.channels_mut()[3] = *text_mask;
                    background.blend(&text_color);
                });
        }
        let width = background.width();
        let height = background.height();
        let buf = Bytes::from(background.into_raw());
        BytesImage::from_raw(width, height, buf).ok_or(anyhow!(
            "Creating BytesImage from flattened RGBA image failed"
        ))
    }
}

impl From<RenderSettings> for ImageLayers {
    fn from(settings: RenderSettings) -> Self {
        let font_renderer = settings.units.map(|_| default_renderer());
        Self {
            background_renderer: ImageBackground::from(&settings),
            font_renderer,
            display: settings.units.into(),
            grid_size: settings.grid_size,
            display_temperature: settings.units.into(),
        }
    }
}
