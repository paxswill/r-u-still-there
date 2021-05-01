// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::Bytes;
use image::{Luma, Rgba};

use std::vec::Vec;

pub type ThermalImage = image::ImageBuffer<Luma<f32>, Vec<f32>>;
pub type BytesImage = image::ImageBuffer<Rgba<u8>, Bytes>;
