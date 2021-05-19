// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::Bytes;
use image::{ImageBuffer, Luma, Rgba};

/// Images where each point is a temperature in degrees Celsius.
pub type ThermalImage = ImageBuffer<Luma<f32>, Vec<f32>>;

/// Rendered raw images intended for viewing. A shared [bytes::Bytes] buffer is used to minimize
/// copying.
pub type BytesImage = ImageBuffer<Rgba<u8>, Bytes>;
