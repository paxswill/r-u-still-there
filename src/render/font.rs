// SPDX-License-Identifier: GPL-3.0-or-later
pub const DEJA_VU_SANS: &[u8] = include_bytes!("DejaVuSans-Numbers.ttf");

pub(super) const FONT_SIZE: f32 = 12.0;

#[cfg(feature = "render_fontdue")]
mod fontdue_inner {
    use std::ops::DerefMut;
    use std::sync::{Arc, Mutex};

    use bytes::Bytes;
    use fontdue::layout::{
        CoordinateSystem, HorizontalAlign, Layout, LayoutSettings, TextStyle, VerticalAlign,
    };
    use fontdue::{layout, Font, FontSettings};

    use image::imageops::overlay;
    use image::{GrayImage, ImageBuffer, Pixel, Rgb, RgbaImage};

    use super::DEJA_VU_SANS;
    use crate::image_buffer::{BytesImage, ThermalImage};
    use crate::render::color;
    use crate::temperature::{Temperature, TemperatureUnit};

    pub(crate) struct FontdueRenderer {
        font: [Font; 1],
        layout: Arc<Mutex<layout::Layout>>,
        grid_size: u32,
    }

    #[cfg(feature = "render_fontdue")]
    impl FontdueRenderer {
        pub(crate) fn new(grid_size: u32) -> Self {
            let font = Font::from_bytes(DEJA_VU_SANS, FontSettings::default()).unwrap();
            Self {
                font: [font],
                layout: Arc::new(Mutex::new(Layout::new(CoordinateSystem::PositiveYDown))),
                grid_size,
            }
        }

        fn reset_layout<'a>(&self, layout: &mut Layout) {
            layout.reset(&LayoutSettings {
                x: 0.0,
                y: 0.0,
                max_height: Some(self.grid_size as f32),
                max_width: Some(self.grid_size as f32),
                horizontal_align: HorizontalAlign::Center,
                vertical_align: VerticalAlign::Middle,
                ..LayoutSettings::default()
            });
        }

        fn render_grid(&self, temperature: Temperature, text_color: color::Color) -> BytesImage {
            let mut layout = self.layout.lock().unwrap();
            self.reset_layout(layout.deref_mut());
            let text = format!("{:.2}", &temperature);
            let style = TextStyle::new(&text, 12.0, 0);
            layout.append(&self.font, &style);
            let mut opacity = GrayImage::new(self.grid_size, self.grid_size);
            let glyphs = layout.glyphs().clone();
            for glyph in glyphs.iter() {
                let (metrics, bitmap) = self.font[0].rasterize_config(glyph.key);
                let bitmap =
                    ImageBuffer::from_vec(metrics.width as u32, metrics.height as u32, bitmap)
                        .expect("the provided buffer to be large enough");
                overlay(&mut opacity, &bitmap, glyph.x as u32, glyph.y as u32)
            }
            // Combine the provided color with the opacity in `cell`. Also expand to an RGBA image
            // at this point.
            let text_color_pixel: Rgb<_> = text_color.as_array().into();
            let mut cell =
                RgbaImage::from_pixel(self.grid_size, self.grid_size, text_color_pixel.to_rgba());
            cell.pixels_mut()
                .zip(opacity.pixels().map(|pixel| pixel.0[0]))
                .for_each(|(pixel, opacity)| {
                    pixel.channels_mut()[3] = opacity;
                });
            ImageBuffer::from_raw(self.grid_size, self.grid_size, Bytes::from(cell.into_raw()))
                .expect("the conversion from a Vec ImageBuffer to a Bytes ImageBuffer to work")
        }

        pub(crate) fn render_text(
            &self,
            units: TemperatureUnit,
            temperatures: &ThermalImage,
            temperature_colors: &RgbaImage,
            grid_image: &mut RgbaImage,
        ) -> anyhow::Result<()> {
            temperatures
                .enumerate_pixels()
                .zip(temperature_colors.pixels())
                // Map the temperature in Celsius to whatever the requested unit is.
                .for_each(|((col, row, temperature_pixel), color_pixel)| {
                    let temperature = Temperature::Celsius(temperature_pixel.0[0]).as_unit(&units);
                    let text_color = color::Color::from(color_pixel).foreground_color();
                    let cell = self.render_grid(temperature, text_color);
                    overlay(
                        grid_image,
                        &cell,
                        col * self.grid_size,
                        row * self.grid_size,
                    )
                });
            Ok(())
        }
    }
}

#[cfg(feature = "render_fontdue")]
pub(crate) use fontdue_inner::FontdueRenderer;
