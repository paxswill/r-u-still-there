use colorous;
use std::cmp::Ordering;
use std::convert::From;
use std::fmt;

#[derive(Clone, Copy, Debug)]
pub struct Color {
    red: u8,
    green: u8,
    blue: u8,
}

impl From<colorous::Color> for Color {
    fn from(other_color: colorous::Color) -> Self {
        Color {
            red: other_color.r,
            green: other_color.g,
            blue: other_color.b,
        }
    }
}

impl fmt::LowerHex for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:x}{:x}{:x}", self.red(), self.green(), self.blue())
    }
}

impl fmt::UpperHex for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:X}{:X}{:X}", self.red(), self.green(), self.blue())
    }
}

impl Color {
    const BLACK: Self = Self { red: u8::MIN, green: u8::MIN, blue: u8::MIN };

    const WHITE: Self = Self { red: u8::MAX, green: u8::MAX, blue: u8::MAX };

    pub fn red(&self) -> u8 {
        self.red
    }

    pub fn green(&self) -> u8 {
        self.green
    }

    pub fn blue(&self) -> u8 {
        self.blue
    }

    /// The red component expressed as a value between 0.0 and 1.0.
    pub fn red_unit(&self) -> f32 {
        self.red as f32 / u8::MAX as f32
    }

    /// The green component expressed as a value between 0.0 and 1.0.
    pub fn green_unit(&self) -> f32 {
        self.green as f32 / u8::MAX as f32
    }

    /// The blue component expressed as a value between 0.0 and 1.0.
    pub fn blue_unit(&self) -> f32 {
        self.blue as f32 / u8::MAX as f32
    }

    /// The relative luminance of the color in the sRGB colorspace, as [defined by the
    /// W3C][wsc-lum].
    /// [w3c-lum]: https://www.w3.org/TR/2008/REC-WCAG20-20081211/#relativeluminancedef
    pub fn luminance(&self) -> f32 {
        let colors = [self.red_unit(), self.green_unit(), self.blue_unit()];
        let colors = colors.iter().map( |c| {
            if *c <= 0.3928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        let scaling_coefficients = [0.2126, 0.7152, 0.0722];
        scaling_coefficients.iter().zip(colors).map(|(l, r)| l * r).sum()
    }

    /// Calculate the contrast ratio between this color and another one using the
    /// [W3C definition][w3c-contrast].
    /// [w3c-contrast]: https://www.w3.org/TR/WCAG20/#contrast-ratiodef
    pub fn contrast_ratio(&self, other: &Self) -> f32 {
        // Using lum, and "a" and "b" instead of l_1/l_2 because lowercase "L" and the number
        // "1" can be hard to tell apart with some fonts.
        let other_lum = other.luminance();
        let lum = self.luminance();
        // Lighter luminance means a higher value.
        if other_lum > lum {
            (other_lum + 0.05) / (lum + 0.05)
        } else if lum > other_lum {
            (lum + 0.05) / (other_lum + 0.05)
        } else {
            // identical luminosity, so no contrast
            1.0
        }
    }

    /// Treating this color as the background, pick a color from the given colors with the highest
    /// contrast ratio.
    pub fn text_color(&self, text_colors: &[Color]) -> Self {
        let possible_color = text_colors.iter().map(|c| (c, self.contrast_ratio(c))).max_by(|l, r| {
            if l.1.is_nan() {
                Ordering::Greater
            } else if r.1.is_nan() {
                Ordering::Less
            } else {
                l.1.partial_cmp(&r.1).unwrap()
            }
        });
        match possible_color {
            Some((color, _)) => *color,
            None => {
                // If there isn't a max, an empty slice was given. Choose from white and black
                // instead.
                self.text_color(&[
                    Self::WHITE,
                    Self::BLACK,
                ])
            }
        }

    }
}