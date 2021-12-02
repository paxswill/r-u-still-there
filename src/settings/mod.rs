// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

mod cli;
pub(crate) mod gradient;

use crate::camera::CameraSettings;
use crate::mqtt::MqttSettings;
use crate::occupancy::TrackerSettings;
use crate::render::RenderSettings;
use crate::stream::StreamSettings;
pub(crate) use cli::Args;

#[derive(Debug, Deserialize, PartialEq)]
pub(crate) struct Settings {
    /// Camera-specific settings.
    pub(crate) camera: CameraSettings,

    /// Settings related to the HTTP server for the video streams.
    #[serde(default)]
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
