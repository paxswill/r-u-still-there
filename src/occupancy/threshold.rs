// SPDX-License-Identifier: GPL-3.0-or-later
use image::{ImageBuffer, Luma, Primitive};
use num_traits::{Bounded, Zero};
use serde::Deserialize;

use crate::image_buffer::ThermalImage;
use crate::temperature::Temperature;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum Threshold {
    Static(Temperature),
    Automatic,
}

const AUTOMATIC_THRESHOLD_DIFFERENCE: f32 = 0.001;

fn automatic_threshold(image: &ThermalImage, current_threshold: Option<f32>) -> f32 {
    let threshold = current_threshold
        .unwrap_or_else(|| image.iter().sum::<f32>() / (image.height() * image.width()) as f32);
    let mut background = Vec::<f32>::default();
    let mut foreground = Vec::<f32>::default();
    for pixel in image.iter() {
        if pixel >= &threshold {
            foreground.push(*pixel);
        } else {
            background.push(*pixel);
        }
    }
    let background_count = background.len() as f32;
    let background_mean: f32 = background.into_iter().sum::<f32>() / background_count;
    let foreground_count = foreground.len() as f32;
    let foreground_mean: f32 = foreground.into_iter().sum::<f32>() / foreground_count;
    let new_threshold = (foreground_mean + background_mean) / 2.0;
    if (new_threshold - threshold).abs() <= AUTOMATIC_THRESHOLD_DIFFERENCE {
        threshold
    } else {
        automatic_threshold(image, Some(new_threshold))
    }
}

impl Threshold {
    fn calculate_level(&self, image: &ThermalImage) -> f32 {
        match self {
            Self::Static(n) => n.in_celsius(),
            Self::Automatic => automatic_threshold(image, None),
        }
    }

    pub(crate) fn threshold_image<T>(&self, image: &ThermalImage) -> ImageBuffer<Luma<T>, Vec<T>>
    where
        T: 'static + Bounded + Zero + Primitive,
    {
        let threshold = self.calculate_level(image);
        let mut threshold_image = ImageBuffer::new(image.width(), image.height());
        let pixel_pairs = image.iter().zip(threshold_image.iter_mut());
        for (source_pixel, threshold_pixel) in pixel_pairs {
            *threshold_pixel = if source_pixel < &threshold {
                T::zero()
            } else {
                T::max_value()
            };
        }
        threshold_image
    }
}

impl Default for Threshold {
    fn default() -> Self {
        Self::Automatic
    }
}

#[cfg(test)]
mod test {
    use super::{automatic_threshold, Threshold};
    use crate::image_buffer::ThermalImage;
    use image::{ImageBuffer, Luma};

    type ThresholdImage = ImageBuffer<Luma<u32>, Vec<u32>>;

    // Just chose these as small-ish, but also not a square image.
    const WIDTH: u32 = 5;
    const HEIGHT: u32 = 3;

    #[rustfmt::skip::macros(vec)]
    fn image() -> ThermalImage {
        // It's an image, with a rectangular portion in the middle that is "hot".
        // "Hot" pixels have a mean near 28.0, "cold" have a mean around 17.0
        ThermalImage::from_raw(
            WIDTH,
            HEIGHT,
            vec![
                14.0, 20.7, 16.1, 16.1, 17.8,
                15.5, 27.8, 29.6, 28.5, 20.2,
                18.8, 16.3, 19.4, 12.1, 20.6,
            ],
        )
        .unwrap()
    }

    #[rustfmt::skip::macros(vec)]
    fn expected_image() -> ThresholdImage {
        ThresholdImage::from_raw(
            WIDTH,
            HEIGHT,
            vec![
                0, 0, 0, 0, 0,
                0, u32::MAX, u32::MAX, u32::MAX, 0,
                0, 0, 0, 0, 0,
            ],
        )
        .unwrap()
    }

    #[test]
    fn dimensions() {
        let image = image();
        let processed: ThresholdImage = Threshold::default().threshold_image(&image);
        assert_eq!(image.dimensions(), processed.dimensions());
    }

    #[test]
    fn apply_static_threshold() {
        let image = image();
        // Choosing 22.5 as it's the midpoint between the mean hot and cold points.
        let processed = Threshold::Static(22.5.into()).threshold_image(&image);
        assert_eq!(processed, expected_image());
    }

    #[test]
    fn apply_automatic_threshold() {
        let image = image();
        let processed = Threshold::Automatic.threshold_image(&image);
        assert_eq!(processed, expected_image());
    }

    #[test]
    fn find_automatic() {
        // Calculated externally
        let auto_threshold = automatic_threshold(&image(), None);
        let expected: f32 = 22.96667;
        assert!((auto_threshold - expected) < 0.0001)
    }
}
