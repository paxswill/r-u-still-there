// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

mod camera;
mod i2c;

pub use camera::{CameraOptions, CameraSettings};
pub use i2c::I2cSettings;

#[derive(Copy, Clone, Debug, Deserialize)]
pub struct Settings<'a> {
    #[serde(borrow)]
    pub camera: CameraSettings<'a>,
}