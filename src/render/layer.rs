// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::TryFrom;

use anyhow::anyhow;
use bytes::Bytes;
use futures::future::{self, FutureExt};
use image::{Pixel, Rgba};

use crate::camera::Measurement;
use crate::image_buffer::BytesImage;

use super::color::Color;
use super::color_map::{ColorMapper, ImageColorMap};
use super::font::{default_renderer, FontRenderer};
use super::resize::{Resizer, PointResize};
use super::settings::RenderSettings;
use super::TemperatureDisplay;

#[derive(Debug)]
pub(crate) struct ImageLayers {
    color_mapper: Box<dyn ColorMapper + Send + Sync>,
    resizer: Box<dyn Resizer + Send + Sync>,
    font_renderer: Option<Box<dyn FontRenderer + Send + Sync>>,
    display: TemperatureDisplay,
    grid_size: usize,
    display_temperature: TemperatureDisplay,
}

impl ImageLayers {
    pub(crate) async fn render(&self, measurement: Measurement) -> anyhow::Result<BytesImage> {
        // Cloning the measurement is (comparatively) cheap, as the thermal image is tucked behind
        // an Arc
        // TODO: figure out a way to do the color mapping asynchronously
        let colors = self.color_mapper.render(measurement.clone()).await?;
        let background_task = self.resizer.enlarge(colors);
        let font_task = match self.display_temperature {
            TemperatureDisplay::Disabled => future::ok(None).boxed(),
            // Look at the size of that...
            TemperatureDisplay::Absolute(unit) => {
                let font_renderer = self.font_renderer.as_ref().ok_or(anyhow!(
                    "Font renderer not created for displayed temperature units"
                ))?;
                font_renderer
                    .render(self.grid_size, unit, measurement.clone())
                    .map(|text| Some(text).transpose())
                    .boxed()
            }
        };
        let (mut background, font_layer_result) = futures::join!(background_task, font_task);
        let font_layer = font_layer_result?;
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

impl TryFrom<RenderSettings> for ImageLayers {
    type Error = anyhow::Error;

    fn try_from(settings: RenderSettings) -> anyhow::Result<Self> {
        let font_renderer = settings.units.map(|_| default_renderer());
        let resizer = PointResize::try_from(&settings)?;
        Ok(Self {
            color_mapper: Box::new(ImageColorMap::from(&settings)),
            resizer: Box::new(resizer),
            font_renderer,
            display: settings.units.into(),
            grid_size: settings.grid_size,
            display_temperature: settings.units.into(),
        })
    }
}
