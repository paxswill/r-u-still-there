// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

mod camera;
mod cli;
mod gradient;
mod i2c;
mod render;
mod stream;

pub use camera::{CameraSettings, CommonOptions, Rotation};
pub use cli::Args;
pub use i2c::I2cSettings;
pub use render::RenderSettings;
pub use stream::StreamSettings;

#[derive(Debug, Deserialize)]
pub struct Settings {
    /// A dummy field, present when parsing CLI arguments.
    #[serde(default)]
    config_path: Option<String>,

    /// Camera-specific settings.
    pub camera: CameraSettings,

    /// Settings related to the HTTP server for the video streams.
    pub streams: StreamSettings,

    /// Settings related to how the data is rendered for the video streams.
    #[serde(default)]
    pub render: RenderSettings,
}
