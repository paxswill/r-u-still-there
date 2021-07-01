// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::Bytes;
use image::Pixel;
use serde::Deserialize;
use tracing::{debug, instrument, trace};

use crate::image_buffer::{BytesImage, ThermalImage};
use crate::temperature::TemperatureUnit;

pub mod color;
pub mod font;

#[cfg(feature = "render_fontdue")]
mod cheese;
#[cfg(feature = "render_svg")]
mod svg;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Limit {
    /// Set the maximum (or minimum) to the largest (or smallest) value in the current image.
    Dynamic,

    /// Set the maximum (or minimum) to the given value.
    Static(f32),
}

impl Default for Limit {
    fn default() -> Self {
        Self::Dynamic
    }
}

/// Control how the temperature of each pixel is displayed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TemperatureDisplay {
    /// Don't show the temperature.
    Disabled,

    /// Display the absolute temperature in the given units.
    Absolute(TemperatureUnit),
}

impl Default for TemperatureDisplay {
    fn default() -> Self {
        Self::Disabled
    }
}
#[derive(Debug)]
pub struct Renderer {
    scale_min: Limit,
    scale_max: Limit,
    display_temperature: TemperatureDisplay,
    grid_size: usize,
    gradient: colorous::Gradient,
}

impl Renderer {
    /// Creates a new `Renderer`. If [Static][Limit::Static] limits are being used for both values
    /// and are in reverse order (i.e. the minimum is larger than the maximum) the color scale will
    /// be reversed. There is not a way to specify this behavior for [Dynamic][Limit::Dynmanic]
    /// limits.
    pub(crate) fn new(
        scale_min: Limit,
        scale_max: Limit,
        display_temperature: TemperatureDisplay,
        grid_size: usize,
        gradient: colorous::Gradient,
    ) -> Self {
        Renderer {
            scale_min,
            scale_max,
            display_temperature,
            grid_size,
            gradient,
        }
    }

    /// Render an image to a pixel buffer.
    #[instrument(level = "debug", skip(image))]
    pub(crate) fn render_buffer(&self, image: &ThermalImage) -> BytesImage {
        // Map the thermal image to an actual RGB image. We're converting to RGBA at the same time
        // as that's what resvg wants.
        let map_func = self.color_map(image);
        let source_width = image.width();
        let source_height = image.height();
        let mut temperature_colors = image::RgbaImage::new(source_width, source_height);
        for (source, dest) in image.pixels().zip(temperature_colors.pixels_mut()) {
            *dest = image::Rgb::from(map_func(&source.0[0]).as_array()).to_rgba();
        }
        trace!("mapped temperatures to colors");
        let mut rgba_image = self.enlarge_color_image(&temperature_colors);
        let full_width = rgba_image.width();
        let full_height = rgba_image.height();
        trace!(
            source_width,
            source_height,
            enlarged_width = full_width,
            enlarged_height = full_height,
            "enlarged source image"
        );
        match self.display_temperature {
            TemperatureDisplay::Disabled => debug!("not rendering temperatures"),
            // Look at the size of it!
            TemperatureDisplay::Absolute(units) => {
                svg::render_text(
                    self.grid_size,
                    units,
                    &image,
                    &temperature_colors,
                    &mut rgba_image,
                ).expect("Rendering text to work");
                debug!("rendered temperatures onto image");
            }
        }
        let buf = Bytes::from(rgba_image.into_raw());
        BytesImage::from_raw(full_width, full_height, buf).unwrap()
    }

    fn color_map(&self, image: &ThermalImage) -> Box<dyn Fn(&f32) -> color::Color> {
        let scale_min = match self.scale_min {
            Limit::Static(n) => n,
            Limit::Dynamic => {
                *(image
                    .iter()
                    .filter(|n| !n.is_nan())
                    .min_by(|l, r| l.partial_cmp(&r).unwrap())
                    .unwrap())
            }
        };
        let scale_max = match self.scale_max {
            Limit::Static(n) => n,
            Limit::Dynamic => {
                *(image
                    .iter()
                    .filter(|n| !n.is_nan())
                    .max_by(|l, r| l.partial_cmp(&r).unwrap())
                    .unwrap())
            }
        };
        let scale_range = scale_max - scale_min;
        // Clone the gradient so that it can be owned by the closure
        let gradient = self.gradient;
        Box::new(move |temperature: &f32| -> color::Color {
            color::Color::from(
                gradient.eval_continuous(((temperature - scale_min) / scale_range) as f64),
            )
        })
    }

    /// This is a fast way to enlarge a grid of individual pixels. Each input pixel will be
    /// enlarged to a `grid_size` square.
    ///
    /// The current implementation builds a series of mono-color image views (using
    /// [image::flat::FlatSamples::with_monocolor]), then drawing these grid squares on to the
    /// final image using [image::imageops::replace]. Alternative implementations that were tested
    /// include:
    ///
    /// * [image::imageops::resize] with [nearest neighbor][image::imageops::FilterType::Nearest]
    ///   filtering. This seems to increase runtime exponentially; with a 30 pixel grid size, a
    ///   BeagleBone Black/Green could (barely) keep up with a 10 FPS GridEYE image, but at 50
    ///   pixels would lag to roughly 2 FPS.
    /// * Duplicating individual pixels using `flat_map`, `repeat` and `take`, then `collect`ing
    ///   everything into a vector. This was faster than `resize`, but still not fast enough.
    /// * As above, but pre-allocating the vector. No significant change.
    ///
    /// With this implementation, a BeagleBone Black/Green can server up an MJPEG stream with 50
    /// pixel grid squares at 10 FPS while keeping CPU usage below 50%.
    fn enlarge_color_image<I, P>(&self, colors: &I) -> image::ImageBuffer<P, Vec<P::Subpixel>>
    where
        I: image::GenericImageView<Pixel = P>,
        P: Pixel + 'static,
        P::Subpixel: 'static,
    {
        let grid_size = self.grid_size as u32;
        let mut full_image =
            image::ImageBuffer::new(colors.width() * grid_size, colors.height() * grid_size);
        for (x, y, pixel) in colors.pixels() {
            let tile = image::flat::FlatSamples::with_monocolor(&pixel, grid_size, grid_size);
            let tile_view = tile.as_view().unwrap();
            image::imageops::replace(&mut full_image, &tile_view, x * grid_size, y * grid_size);
        }
        full_image
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new(
            Limit::Dynamic,
            Limit::Dynamic,
            TemperatureDisplay::default(),
            50,
            colorous::TURBO,
        )
    }
}

#[cfg(test)]
mod color_map_tests {
    use super::{color, Limit, Renderer, TemperatureDisplay};
    use crate::image_buffer::ThermalImage;

    lazy_static! {
        // Ensure values outside of the static limits (0 and 100) are tested.
        static ref TEST_IMAGE: ThermalImage = ThermalImage::from_vec(
            6, 1,
            vec![-25.0, 0.0, 25.0, 50.0, 75.0, 150.0]
        ).unwrap();
    }

    #[test]
    fn both_static() {
        // range is from 0 to 100
        test_limits(
            Limit::Static(0.0),
            Limit::Static(100.0),
            [0.0, 0.0, 0.25, 0.5, 0.75, 1.0],
        );
    }

    #[test]
    fn upper_dynamic() {
        // range is from 0 to 150
        test_limits(
            Limit::Static(0.0),
            Limit::Dynamic,
            [0.0, 0.0, (1.0 / 6.0), (1.0 / 3.0), 0.5, 1.0],
        );
    }

    #[test]
    fn lower_dynamic() {
        test_limits(
            // Range is from -25 to 100
            Limit::Dynamic,
            Limit::Static(100.0),
            [0.0, 0.2, 0.4, 0.6, 0.8, 1.0],
        );
    }

    #[test]
    fn both_dynamic() {
        test_limits(
            // Range is from -25 to 150
            Limit::Dynamic,
            Limit::Dynamic,
            // Most of these values are irrational
            [
                0.0,
                25.0 / 175.0,
                50.0 / 175.0,
                75.0 / 175.0,
                100.0 / 175.0,
                1.0,
            ],
        );
    }

    #[test]
    fn reversed_static() {
        // range is from 0 to 100
        test_limits(
            Limit::Static(100.0),
            Limit::Static(0.0),
            [1.0, 1.0, 0.75, 0.5, 0.25, 0.0],
        );
    }

    // Putting this below the actual usage of the tests to make it easier to visually reference the
    // test values.
    fn test_limits(lower_limit: Limit, upper_limit: Limit, expected: [f64; 6]) {
        let renderer = Renderer::new(
            lower_limit,
            upper_limit,
            TemperatureDisplay::Disabled,
            10,
            colorous::GREYS,
        );
        // Ensure values outside of the static limits (0 and 100) are tested.
        let map_func = renderer.color_map(&TEST_IMAGE);
        for (pixel, expected) in TEST_IMAGE.iter().zip(&expected) {
            let mapped = map_func(pixel);
            let expected_color = color::Color::from(colorous::GREYS.eval_continuous(*expected));
            assert_eq!(
                mapped, expected_color,
                "mapped {:?} to {:?}, but expected {:?} (from {:?})",
                pixel, mapped, expected_color, expected
            );
        }
    }
}
