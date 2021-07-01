// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::anyhow;
use image::RgbaImage;
use svg::node::element::{Group, Rectangle, Text as TextElement};
use svg::node::Text as TextNode;
use svg::Document;
use tiny_skia::PixmapMut;
use tracing::instrument;
use usvg::{FitTo, Tree};

use crate::image_buffer::ThermalImage;
use crate::temperature::{Temperature, TemperatureUnit};

use super::{color, font};

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

/// Create an SVG document to render the temperatures of a thermal image.
fn create_document(
    // Using u32 as that's what resvg/tiny_skia use for sizes.
    grid_size: u32,
    units: TemperatureUnit,
    temperatures: &ThermalImage,
    temperature_colors: &image::RgbaImage,
) -> Document {
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
        .map(|((col, row, temperature_pixel), color_pixel)| {
            let text_color = color::Color::from(color_pixel).foreground_color();
            let grid_cell = Rectangle::new()
                .set("fill", "none")
                .set("width", grid_size)
                .set("height", grid_size)
                .set("x", col * grid_size)
                .set("y", row * grid_size);
            let group = Group::new().add(grid_cell);
            // unwrap the actual temperature from the Luma pixel
            let temperature = temperature_pixel.0[0];
            let mapped_temperature = match units {
                TemperatureUnit::Celsius => temperature,
                TemperatureUnit::Fahrenheit => Temperature::Celsius(temperature).in_fahrenheit(),
            };
            group.add(
                TextElement::new()
                    .set("fill", format!("{:X}", text_color))
                    .set("text-anchor", "middle")
                    .set("font-size", format!("{}px", font::FONT_SIZE))
                    // resvg doesn't support dominant-baseline yet, so it gets rendered
                    // incorrectly for the time being.
                    .set("dominant-baseline", "middle")
                    .set("x", col * grid_size + (grid_size / 2))
                    .set("y", row * grid_size + (grid_size / 2))
                    .add(TextNode::new(format!("{:.2}", mapped_temperature))),
            )
        })
        .fold(Document::new(), |doc, group| doc.add(group))
        .set("width", temperatures.width() * grid_size)
        .set("height", temperatures.height() * grid_size)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SvgRenderer();

impl font::FontRenderer for SvgRenderer {
    /// Draw the text for temperatures on top of an existing grid.
    #[instrument(
        level = "trace",
        skip(grid_size, units, temperatures, temperature_colors, grid_image)
    )]
    fn render_text(
        &self,
        grid_size: usize,
        units: TemperatureUnit,
        temperatures: &ThermalImage,
        temperature_colors: &RgbaImage,
        grid_image: &mut RgbaImage,
    ) -> anyhow::Result<()> {
        let width = grid_image.width();
        let height = grid_image.height();
        let mut samples = grid_image.as_flat_samples_mut();
        let image_slice = samples.image_mut_slice().ok_or(anyhow!(
            "Unable to access a mutable slice of the grid image data"
        ))?;
        let pixmap = PixmapMut::from_bytes(image_slice, width, height)
            .ok_or(anyhow!("Unable to create Pixmap for SVG text rendering"))?;
        let svg = create_document(grid_size as u32, units, temperatures, temperature_colors);
        let tree = Tree::from_str(&svg.to_string(), &SVG_OPTS)?;
        resvg::render(&tree, FitTo::Original, pixmap)
            .ok_or(anyhow!("Unable to render SVG for text rendering."))?;
        Ok(())
    }
}
