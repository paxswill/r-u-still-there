// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

mod cli;
pub(crate) mod gradient;
mod stream;
mod tracker;

use crate::camera::CameraSettings;
use crate::mqtt::MqttSettings;
use crate::render::RenderSettings;
pub(crate) use cli::Args;
pub(crate) use stream::StreamSettings;
pub(crate) use tracker::TrackerSettings;

#[derive(Debug, Deserialize)]
pub(crate) struct Settings {
    /// A dummy field, present when parsing CLI arguments.
    #[serde(default)]
    config_path: Option<String>,

    /// Camera-specific settings.
    pub(crate) camera: CameraSettings,

    /// Settings related to the HTTP server for the video streams.
    pub(crate) streams: StreamSettings,

    /// Settings related to how the data is rendered for the video streams.
    #[serde(default)]
    pub(crate) render: RenderSettings,

    /// Occupancy tracker settings.
    #[serde(default)]
    pub(crate) tracker: TrackerSettings,

    /// MQTT server connection settings.
    pub(crate) mqtt: MqttSettings,
}
