// SPDX-License-Identifier: GPL-3.0-or-later
use std::cmp::Ordering;
use std::convert::From;
use std::fmt;

/// A type for colors specifically for finding corresponding colors that have good contrast.
/// This type uses the WCAG 2.0 definitions of "relative luminance" and "contrast ratio". These
/// definitions are not very good, but they're good enough for our purposes.
///
/// This type can be formatted as a hex code using the standard formatting syntax. The formatted
/// output will have a leading '#'.
#[derive(Clone, Copy, Debug, PartialEq)]
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

impl From<&image::Rgb<u8>> for Color {
    fn from(pixel: &image::Rgb<u8>) -> Self {
        Self::new(pixel[0], pixel[1], pixel[2])
    }
}

impl From<&image::Rgba<u8>> for Color {
    fn from(pixel: &image::Rgba<u8>) -> Self {
        Self::new(pixel[0], pixel[1], pixel[2])
    }
}

impl From<Color> for [u8; 3] {
    fn from(color: Color) -> Self {
        color.as_array()
    }
}

impl From<Color> for (u8, u8, u8) {
    fn from(color: Color) -> Self {
        color.as_tuple()
    }
}

impl fmt::LowerHex for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{:02x}{:02x}{:02x}",
            self.red(),
            self.green(),
            self.blue()
        )
    }
}

impl fmt::UpperHex for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{:02X}{:02X}{:02X}",
            self.red(),
            self.green(),
            self.blue()
        )
    }
}

impl Color {
    const BLACK: Self = Self {
        red: u8::MIN,
        green: u8::MIN,
        blue: u8::MIN,
    };

    const WHITE: Self = Self {
        red: u8::MAX,
        green: u8::MAX,
        blue: u8::MAX,
    };

    /// Create a new [Color] with the given 8-bit color values.
    pub fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// The 8-bit color value of the red component.
    pub fn red(&self) -> u8 {
        self.red
    }

    /// The 8-bit color value of the green component.
    pub fn green(&self) -> u8 {
        self.green
    }

    /// The 8-bit color value of the blue component.
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
        let colors = colors.iter().map(|c| {
            // NOTE: 0.03928 is an error from a draft sRGB spec from the W3C. 0.04045 is the
            // correct value.
            if *c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        let scaling_coefficients = [0.2126, 0.7152, 0.0722];
        scaling_coefficients
            .iter()
            .zip(colors)
            .map(|(l, r)| l * r)
            .sum()
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

    /// Treating this color as the background, return a color with a decent contrast ratio.
    ///
    /// The possible colors this method can return is an implementation detail, but will generally
    /// be a neutral color. At this point in time white and black used, but others may be added
    /// later.
    pub fn foreground_color(&self) -> Self {
        self.foreground_color_custom(&[Self::WHITE, Self::BLACK])
    }

    /// Treating this color as the background, pick a color from the given colors with the highest
    /// contrast ratio.
    pub fn foreground_color_custom(&self, text_colors: &[Color]) -> Self {
        let possible_color = text_colors
            .iter()
            .map(|c| (c, self.contrast_ratio(c)))
            .max_by(|l, r| {
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
                // If there isn't a max, an empty slice was given. Use the default colors instead
                self.foreground_color()
            }
        }
    }

    /// The red, green, and blue components as a 3-tuple.
    pub fn as_tuple(&self) -> (u8, u8, u8) {
        (self.red(), self.green(), self.blue())
    }

    /// The red, green, and blue components as a 3 element array.
    pub fn as_array(&self) -> [u8; 3] {
        [self.red(), self.green(), self.blue()]
    }
}

#[cfg(test)]
mod color_test {
    use super::Color;
    use float_cmp::{approx_eq, F32Margin};

    #[test]
    fn black() {
        let black = Color::BLACK;
        assert_eq!(black.red(), 0);
        assert_eq!(black.green(), 0);
        assert_eq!(black.blue(), 0);
    }

    #[test]
    fn white() {
        let white = Color::WHITE;
        assert_eq!(white.red(), u8::MAX);
        assert_eq!(white.green(), u8::MAX);
        assert_eq!(white.blue(), u8::MAX);
    }

    #[test]
    fn new_order() {
        let c = Color::new(25, 125, 225);
        assert_eq!(c.red(), 25);
        assert_eq!(c.green(), 125);
        assert_eq!(c.blue(), 225);
    }

    #[test]
    fn unit() {
        let c = Color::new(51, 170, 255);
        assert!(approx_eq!(f32, c.red_unit(), 0.2, F32Margin::default()));
        assert!(approx_eq!(
            f32,
            c.green_unit(),
            2.0 / 3.0,
            F32Margin::default()
        ));
        assert!(approx_eq!(f32, c.blue_unit(), 1.0, F32Margin::default()));
    }

    #[test]
    fn luminance() {
        assert!(approx_eq!(
            f32,
            Color::BLACK.luminance(),
            0.0,
            F32Margin::default()
        ));
        assert!(approx_eq!(
            f32,
            Color::WHITE.luminance(),
            1.0,
            F32Margin::default()
        ));
    }

    mod contrast_ratio {
        use super::Color;
        use float_cmp::{approx_eq, F32Margin};

        const WHITE: Color = Color::WHITE;
        const BLACK: Color = Color::BLACK;
        const RED: Color = Color {
            red: u8::MAX,
            green: 0,
            blue: 0,
        };
        const GREEN: Color = Color {
            red: 0,
            green: u8::MAX,
            blue: 0,
        };
        const BLUE: Color = Color {
            red: 0,
            green: 0,
            blue: u8::MAX,
        };

        #[test]
        fn limits() {
            // The definition of constrast ratio we're using ranges from 1 to 21
            assert!(approx_eq!(
                f32,
                WHITE.contrast_ratio(&BLACK),
                21.0,
                F32Margin::default()
            ));
            assert_eq!(WHITE.contrast_ratio(&WHITE), 1.0);
            assert_eq!(BLACK.contrast_ratio(&BLACK), 1.0);
        }

        #[test]
        fn webaim_definitions() {
            // The following values are referenced from https://webaim.org/articles/contrast/
            assert!(approx_eq!(
                f32,
                RED.contrast_ratio(&WHITE),
                4.0,
                epsilon = 0.01
            ));
            assert!(approx_eq!(
                f32,
                BLUE.contrast_ratio(&WHITE),
                8.6,
                epsilon = 0.01
            ));
            // Not using the example for green as their values are rounded (while they
            // mention not to round later on in that document with the example we're using here).
            let light_gray = Color::new(0x77, 0x77, 0x77);
            assert!(approx_eq!(
                f32,
                light_gray.contrast_ratio(&WHITE),
                4.47,
                epsilon = 0.01
            ));
        }

        /// Check that the order doesn't matter (more accurately, that the light value is chosen
        /// properly).
        #[test]
        fn symmetric() {
            assert_eq!(RED.contrast_ratio(&WHITE), WHITE.contrast_ratio(&RED));
            assert_eq!(GREEN.contrast_ratio(&WHITE), WHITE.contrast_ratio(&GREEN));
            assert_eq!(BLUE.contrast_ratio(&WHITE), WHITE.contrast_ratio(&BLUE));
            assert_eq!(BLACK.contrast_ratio(&WHITE), WHITE.contrast_ratio(&BLACK));
        }
    }

    #[test]
    fn text_default() {
        let yellow = Color::new(0xFF, 0xFF, 0x47);
        assert_eq!(yellow.foreground_color(), Color::BLACK);
        let purple = Color::new(0x77, 0, 0xFF);
        assert_eq!(purple.foreground_color(), Color::WHITE);
    }

    #[test]
    fn text_given() {
        let yellow = Color::new(0xFF, 0xFF, 0x47);
        let purple = Color::new(0x77, 0, 0xFF);
        let cyan = Color::new(0, 0xFF, 0xFF);
        let dark_orange = Color::new(0x8F, 0x4E, 0x11);
        assert_eq!(purple.foreground_color_custom(&[purple, yellow, dark_orange]), yellow);
        assert_eq!(yellow.foreground_color_custom(&[dark_orange, cyan, yellow]), dark_orange);
    }
}
