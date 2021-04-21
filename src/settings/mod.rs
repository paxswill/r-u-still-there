// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

mod camera;
mod i2c;
mod stream;

pub use camera::{CameraOptions, CameraSettings};
pub use i2c::I2cSettings;
pub use stream::StreamSettings;

#[derive(Debug, Deserialize)]
pub struct Settings<'a> {
    #[serde(borrow)]
    pub camera: CameraSettings<'a>,

    pub streams: StreamSettings,
}
