// SPDX-License-Identifier: GPL-3.0-or-later
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use bytes::Bytes;
use futures::future::{self, FutureExt};
use image::{Pixel, Rgba};
use tokio::task::spawn_blocking;

use crate::camera::Measurement;
use crate::image_buffer::BytesImage;
use crate::util::flatten_join_result;

use super::background::{BackgroundRenderer, ImageBackground};
use super::color::Color;
use super::font::{default_renderer, FontRenderer};
use super::settings::RenderSettings;
use super::TemperatureDisplay;

#[derive(Clone, Debug)]
pub(crate) struct ImageLayers {
    background_renderer: Arc<Mutex<ImageBackground>>,
    font_renderer: Option<Arc<Mutex<Box<dyn FontRenderer + Send + Sync>>>>,
    display: TemperatureDisplay,
    grid_size: usize,
    display_temperature: TemperatureDisplay,
}

impl ImageLayers {
    pub(crate) async fn render(&self, measurement: Measurement) -> anyhow::Result<BytesImage> {
        // Turn the measurement into a refcounted reference so it doesn't need to be copied everywhere
        let measurement = Arc::new(measurement);
        let bg_renderer = Arc::clone(&self.background_renderer);
        let bg_measurement = Arc::clone(&measurement);
        let background_task =
            spawn_blocking(move || bg_renderer.lock().unwrap().render(&bg_measurement));
        let font_task = match self.display_temperature {
            TemperatureDisplay::Disabled => future::ok(None).boxed(),
            TemperatureDisplay::Absolute(unit) => {
                let renderer_arc = self.font_renderer.as_ref().ok_or(anyhow!(
                    "Font renderer not created for displayed temperature units"
                ))?;
                let font_renderer = Arc::clone(renderer_arc);
                let font_measurement = Arc::clone(&measurement);
                let grid_size = self.grid_size;
                spawn_blocking(move || {
                    let font_mask = font_renderer.lock().unwrap().render_text(
                        grid_size,
                        unit,
                        &font_measurement.image,
                    )?;
                    anyhow::Result::<Option<_>>::Ok(Some(font_mask))
                })
                .map(flatten_join_result)
                .boxed()
            }
        };
        let (background_result, font_layer_result) = futures::join!(background_task, font_task);
        let mut background = background_result?;
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

impl From<RenderSettings> for ImageLayers {
    fn from(settings: RenderSettings) -> Self {
        let font_renderer = settings
            .units
            .map(|_| Arc::new(Mutex::new(default_renderer())));
        Self {
            background_renderer: Arc::new(Mutex::new(ImageBackground::from(&settings))),
            font_renderer,
            display: settings.units.into(),
            grid_size: settings.grid_size,
            display_temperature: settings.units.into(),
        }
    }
}
