use ndarray::{Array2, Axis, Zip};
use svg::node::element::Text as TextElement;
use svg::node::element::{Group, Rectangle};
use svg::node::Text as TextNode;
use svg::Document;
use tiny_skia::Pixmap;
use usvg::{FitTo, Tree};

mod color;

lazy_static! {
    /// A basic SVG options structure configured to use the bundled DejaVu Sans font.
    static ref SVG_OPTS: usvg::Options = {
        let mut opts = usvg::Options::default();
        // Add the super stripped down DejaVu Sans (it only has the characters needed to render
        // numbers).
        opts.fontdb.load_font_data(include_bytes!("DejaVuSans-Numbers.ttf").to_vec());
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Limit {
    /// Set the maximum (or minimum) to the largest (or smallest) value in the current image.
    Dynamic,

    /// Set the maximum (or minimum) to the given value.
    Static(f32),
}

/// Control how the temperature of each pixel is displayed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TemperatureDisplay {
    /// Don't show the temperature.
    Disabled,

    /// Display the temperature in Celsius.
    Celsius,

    /// Display the temperature in fahrenheit.
    Fahrenheit,
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
    pub fn new(
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
            // TODO: implement serde stuff so this can be configurable
            gradient,
        }
    }

    /// Render an image to a pixel buffer.
    pub fn render_buffer(&self, image: &Array2<f32>) -> Pixmap {
        let svg = self.render_svg(image);
        let tree = Tree::from_data(format!("{}", svg).as_bytes(), &SVG_OPTS).unwrap();
        let size = tree.svg_node().size.to_screen_size();
        let mut pixmap = Pixmap::new(size.width(), size.height()).unwrap();
        resvg::render(&tree, FitTo::Original, pixmap.as_mut()).unwrap();
        pixmap
    }

    fn color_map(&self, image: &Array2<f32>) -> Box<dyn Fn(&f32) -> color::Color> {
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
        let gradient = self.gradient.clone();
        Box::new(move |temperature: &f32| -> color::Color {
            color::Color::from(
                gradient.eval_continuous(((temperature - scale_min) / scale_range) as f64),
            )
        })
    }

    /// Create a closure that renders a single value to an SVG group element. The clusre takes a
    /// tuple of the values row and column, the temperature to render, and the color to use for the
    /// background. The size of the grid cell and how to display temperatures is cloned from the
    /// [Renderer] state when this method is called.
    fn render_svg_cell(
        &self,
        row_count: usize,
    ) -> Box<dyn Fn((usize, usize), &f32, &color::Color) -> Group> {
        // Clone some values to be captured by the closure
        let grid_size = self.grid_size;
        let display_temperature = self.display_temperature;
        Box::new(move |(row, col), temperature, grid_color| {
            let text_color = grid_color.text_color(&[]);
            // The SVG coordinate system has the origin in the upper left, while the image's
            // origin is the lower left, so we have to swap them. The row index is otherwise
            // unused, so shadowing it is simplest.
            let row = row_count - row - 1;
            let grid_cell = Rectangle::new()
                // Color implements UpperHex and outputs "#HHHHHH" for the color (like
                // colorous::Color).
                .set("fill", format!("{:X}", grid_color))
                .set("width", grid_size)
                .set("height", grid_size)
                .set("x", col * grid_size)
                .set("y", row * grid_size);
            let group = Group::new().add(grid_cell);
            if display_temperature == TemperatureDisplay::Disabled {
                group
            } else {
                let mapped_temperature = match display_temperature {
                    TemperatureDisplay::Celsius => *temperature,
                    TemperatureDisplay::Fahrenheit => *temperature * 1.8 + 32.0,
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
        })
    }

    pub fn render_svg(&self, image: &Array2<f32>) -> Document {
        let (row_count, col_count) = image.dim();
        let svg_cell_func = self.render_svg_cell(row_count);
        // TODO: investigate parallelizing this
        let grid_colors = Zip::from(image).map_collect(self.color_map(image));
        Zip::indexed(image)
            .and(&grid_colors)
            .fold(Document::new(), |doc, index, temperature, grid_color| {
                doc.add(svg_cell_func(index, temperature, grid_color))
            })
            .set("width", image.len_of(Axis(1)) * self.grid_size)
            .set("height", row_count * self.grid_size)
            .set(
                "viewBox",
                (0, 0, row_count * self.grid_size, col_count * self.grid_size),
            )
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
    use ndarray::{arr2, Array2};

    lazy_static! {
        // Ensure values outside of the static limits (0 and 100) are tested.
        static ref TEST_IMAGE: Array2<f32> = arr2(&[[-25.0, 0.0, 25.0, 50.0, 75.0, 150.0]]);
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
                1.0
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
                mapped,
                color::Color::from(expected_color),
                "mapped {:?} to {:?}, but expected {:?} (from {:?})",
                pixel, mapped, expected_color, expected
            );
        }
    }

}
