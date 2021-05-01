// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::Bytes;
use image::Pixel;
use svg::node::element::{Group, Rectangle, Text as TextElement};
use svg::node::Text as TextNode;
use svg::Document;
use tiny_skia::PixmapMut;
use usvg::{FitTo, Tree};

use crate::image_buffer::{BytesImage, ThermalImage};

use super::{color, font, Limit, Renderer as RendererTrait, TemperatureDisplay};

lazy_static! {
    /// A basic SVG options structure configured to use the bundled DejaVu Sans font.
    static ref SVG_OPTS: usvg::Options = {
        let mut opts = usvg::Options::default();
        // Add the super stripped down DejaVu Sans (it only has the characters needed to render
        // numbers).
        opts.fontdb.load_font_data(font::DEJA_VU_SANS.to_vec());
        opts.font_family = "DejaVu Sans".to_string();
        opts
    };
}

#[cfg(test)]
mod font_tests {
    use super::SVG_OPTS;
    use fontdb::Source;
    use std::fs;
    use std::sync::Arc;
    use ttf_parser::Face;

    #[test]
    fn embedded_font_loaded() {
        let db = &SVG_OPTS.fontdb;
        assert_eq!(db.len(), 1);
        let font = db.faces().iter().next().unwrap();
        assert_eq!(font.family, "DejaVu Sans".to_string());
        assert_eq!(font.style, fontdb::Style::Normal);
        assert_eq!(font.weight, fontdb::Weight::NORMAL);
        assert!(!font.monospaced);
    }

    #[test]
    fn embedded_font_characters() {
        let font_data = {
            let db = &SVG_OPTS.fontdb;
            let font = db.faces().iter().next().unwrap();
            let source = Arc::clone(&font.source);
            match *source {
                Source::Binary(ref bin) => bin.to_owned(),
                Source::File(ref path) => fs::read(path).unwrap(),
            }
        };
        // There better only be one face in the font data.
        let font = Face::from_slice(&font_data, 0);
        assert!(font.is_ok());
        let font = font.unwrap();
        // Leaving the exotic spaces and other localized separators out for now.
        let required_characters = "01223456789-+. ";
        for c in required_characters.chars() {
            assert!(font.glyph_index(c).is_some());
        }
        let discarded_characters = "abcdefghijklmnopqrstuvwxyz";
        for c in discarded_characters.chars() {
            assert!(!font.glyph_index(c).is_some());
        }
    }

    #[test]
    fn default_family() {
        assert_eq!(SVG_OPTS.font_family, "DejaVu Sans");
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

impl RendererTrait for Renderer {
    /// Creates a new `Renderer`. If [Static][Limit::Static] limits are being used for both values
    /// and are in reverse order (i.e. the minimum is larger than the maximum) the color scale will
    /// be reversed. There is not a way to specify this behavior for [Dynamic][Limit::Dynmanic]
    /// limits.
    fn new(
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

    fn scale_min(&self) -> Limit {
        self.scale_min
    }

    fn scale_max(&self) -> Limit {
        self.scale_max
    }

    fn display_temperature(&self) -> TemperatureDisplay {
        self.display_temperature
    }

    fn grid_size(&self) -> usize {
        self.grid_size
    }

    fn set_grid_size(&mut self, grid_size: usize) {
        self.grid_size = grid_size;
    }

    fn gradient(&self) -> colorous::Gradient {
        self.gradient
    }

    fn set_gradient(&mut self, gradient: colorous::Gradient) {
        self.gradient = gradient;
    }

    /// Render an image to a pixel buffer.
    fn render_buffer(&self, image: &ThermalImage) -> BytesImage {
        // Map the thermal image to an actual RGB image. We're converting to RGBA at the same time
        // as that's what resvg wants.
        let map_func = self.color_map(image);
        let mut temperature_colors = image::RgbaImage::new(image.width(), image.height());
        for (source, dest) in image.pixels().zip(temperature_colors.pixels_mut()) {
            *dest = image::Rgb::from(map_func(&source.0[0]).as_array()).to_rgba();
        }
        let mut rgba_image = self.enlarge_color_image(&temperature_colors);
        let full_width = rgba_image.width();
        let full_height = rgba_image.height();
        let buf = if self.display_temperature() != TemperatureDisplay::Disabled {
            let mut pixmap = PixmapMut::from_bytes(
                rgba_image.as_flat_samples_mut().as_mut_slice(),
                full_width,
                full_height,
            )
            .unwrap()
            .to_owned();
            let svg = self.render_text(image, &temperature_colors);
            let tree = Tree::from_data(format!("{}", svg).as_bytes(), &SVG_OPTS).unwrap();
            // Just render on top of the existing data. The generated SVG is just text on a
            // transparent background.
            resvg::render(&tree, FitTo::Original, (pixmap).as_mut()).unwrap();
            Bytes::from(pixmap.take())
        } else {
            Bytes::from(rgba_image.into_raw())
        };
        BytesImage::from_raw(full_width, full_height, buf).unwrap()
    }
}

type ThermalPixel = image::Luma<f32>;

impl Renderer {
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
        let grid_size = self.grid_size() as u32;
        let mut full_image =
            image::ImageBuffer::new(colors.width() * grid_size, colors.height() * grid_size);
        for (x, y, pixel) in colors.pixels() {
            let tile = image::flat::FlatSamples::with_monocolor(&pixel, grid_size, grid_size);
            let tile_view = tile.as_view().unwrap();
            image::imageops::replace(&mut full_image, &tile_view, x * grid_size, y * grid_size);
        }
        full_image
    }

    fn create_svg_fragment(
        &self,
        row: u32,
        col: u32,
        temperature_pixel: &ThermalPixel,
        color_pixel: &image::Rgba<u8>,
    ) -> Group {
        let grid_size = self.grid_size as u32;
        let display_temperature = self.display_temperature;
        let text_color = color::Color::from(color_pixel).text_color(&[]);
        let grid_cell = Rectangle::new()
            .set("fill", "none")
            .set("width", grid_size)
            .set("height", grid_size)
            .set("x", col * grid_size)
            .set("y", row * grid_size);
        let group = Group::new().add(grid_cell);
        if display_temperature == TemperatureDisplay::Disabled {
            group
        } else {
            // unwrap the actual temperature from the Luma pixel
            let temperature = temperature_pixel.0[0];
            let mapped_temperature = match display_temperature {
                TemperatureDisplay::Celsius => temperature,
                TemperatureDisplay::Fahrenheit => temperature * 1.8 + 32.0,
                TemperatureDisplay::Disabled => unreachable!(),
            };
            group.add(
                TextElement::new()
                    .set("fill", format!("{:X}", text_color))
                    .set("text-anchor", "middle")
                    // resvg doesn't support dominant-baseline yet, so it gets rendered
                    // incorrectly for the time being.
                    .set("dominant-baseline", "middle")
                    .set("x", col * grid_size + (grid_size / 2))
                    .set("y", row * grid_size + (grid_size / 2))
                    .add(TextNode::new(format!("{:.2}", mapped_temperature))),
            )
        }
    }

    pub fn render_text(
        &self,
        temperatures: &ThermalImage,
        temperature_colors: &image::RgbaImage,
    ) -> Document {
        let grid_size = self.grid_size as u32;
        // `temperatures` and `temperature_colors` are the same size, and each pixel of
        // `temperature_colors` is the color of the background in that grid square.
        // So the process becomes:
        //   * Zip up the temperature values (from `temperatures`) and background colors (from
        //     `temperature_colors`) with the location of each pixel (provided by
        //     `enumerate_pixels`).
        //   * Map each grouping of those to an SVG fragment.
        //   * Append all of those fragments to a parent SVG document.
        //   * Set a couple of attributes on the parent SVG document to get the size right.
        temperatures
            .enumerate_pixels()
            .zip(temperature_colors.pixels())
            .map(|((x, y, temp), color)| self.create_svg_fragment(x, y, temp, color))
            .fold(Document::new(), |doc, group| doc.add(group))
            .set("width", temperatures.width() * grid_size)
            .set("height", temperatures.height() * grid_size)
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
    use crate::render::Renderer as _;

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
