// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

mod camera;
mod gradient;
mod i2c;
mod render;
mod stream;

pub use camera::{CameraSettings, CommonOptions};
pub use i2c::I2cSettings;
pub use render::RenderSettings;
pub use stream::StreamSettings;

#[derive(Debug, Deserialize)]
pub struct Settings<'a> {
    #[serde(borrow)]
    pub camera: CameraSettings<'a>,

    pub streams: StreamSettings,

    #[serde(default)]
    pub render: RenderSettings,
}
