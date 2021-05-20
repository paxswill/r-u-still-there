// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

mod camera;
mod cli;
mod gradient;
mod i2c;
mod render;
mod stream;
mod tracker;

pub use camera::{CameraSettings, CommonOptions, Rotation};
pub use cli::Args;
pub use i2c::I2cSettings;
pub use render::RenderSettings;
pub use stream::StreamSettings;
pub use tracker::TrackerSettings;

#[derive(Debug, Deserialize)]
pub struct Settings<'a> {
    #[serde(borrow)]
    pub camera: CameraSettings<'a>,

    pub streams: StreamSettings,

    #[serde(default)]
    pub render: RenderSettings,

    #[serde(default)]
    pub tracker: TrackerSettings,
}
