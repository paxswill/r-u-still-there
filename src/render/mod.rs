use colorous;
use svg::Document;
use svg::node::element::{Group,Rectangle};
use svg::node::element::Text as TextElement;
use svg::node::Text as TextNode;
use ndarray::{Axis, Array2};

mod color;

#[derive(Clone, Copy, Debug)]
pub enum Limit {
    /// Set the maximum (or minimum) to the largest (or smallest) value in the current image.
    Dynamic,

    /// Set the maximum (or minimum) to the given value.
    Static(f32),
}

#[derive(Debug)]
pub struct Renderer {
    scale_min: Limit,
    scale_max: Limit,
    show_values: bool,
    grid_size: usize,
    gradient: colorous::Gradient,
}

impl Renderer {
    pub fn new(scale_min: Limit, scale_max: Limit, show_values: bool, grid_size: usize) -> Self {
        Renderer {
            scale_min: scale_min,
            scale_max: scale_max,
            show_values: show_values,
            grid_size: grid_size,
            // Completely static for now.
            // TODO: implement serde stuff so this can be configurable
            gradient: colorous::TURBO,
        }
    }

    pub fn render_svg(&self, image: &Array2<f32>) -> Document {
        let scale_min = match self.scale_min {
            Limit::Static(n) => n,
            Limit::Dynamic => {
                *(image.iter().filter(|n| ! n.is_nan()).min_by(|l, r| {
                    l.partial_cmp(&r).unwrap()
                }).unwrap())
            }
        };
        let scale_max = match self.scale_max {
            Limit::Static(n) => n,
            Limit::Dynamic => {
                *(image.iter().filter(|n| ! n.is_nan()).max_by(|l, r| {
                    l.partial_cmp(&r).unwrap()
                }).unwrap())
            }
        };
        let color_map = |temperature: &f32| -> color::Color {
            color::Color::from(self.gradient.eval_continuous(
                ((temperature - scale_min) / scale_max) as f64
            ))
        };
        let row_count = image.len_of(Axis(0));
        image.indexed_iter().map(|((row, col), temperature)| {
            // The SVG coordinate system has the origin in the upper left, while the image's origin
            // is the lower left, so we have to swap them.
            let svg_row = row_count - row;
            let grid_color = color_map(temperature);
            let text_color = grid_color.text_color(&[]);
            // TODO: pick a text color so it has some contrast with the color
            let grid_cell = Rectangle::new()
                .set("fill", format!("{:X}", grid_color))
                .set("width", self.grid_size)
                .set("height", self.grid_size)
                .set("x", col * self.grid_size)
                .set("y", svg_row * self.grid_size);
            let group = Group::new().add(grid_cell);
            if self.show_values {
                group.add(TextElement::new()
                        .set("fill", format!("{:X}", text_color))
                        .set("text-anchor", "middle")
                        .set("dominant-baseline", "middle")
                        .set("x", col * self.grid_size + (self.grid_size / 2))
                        .set("y", svg_row * self.grid_size + (self.grid_size / 2))
                        .add(
                            TextNode::new(format!("{:.2}", temperature))
                        )
                    )
            } else {
                group
            }
        }).fold(Document::new(), |doc, element| doc.add(element))
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new(Limit::Dynamic, Limit::Dynamic, true, 50)
    }
}
