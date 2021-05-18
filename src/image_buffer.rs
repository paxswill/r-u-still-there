// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::Bytes;
use image::{ImageBuffer, Luma, Rgba};

pub type ThermalImage = ImageBuffer<Luma<f32>, Vec<f32>>;
pub type BytesImage = ImageBuffer<Rgba<u8>, Bytes>;
