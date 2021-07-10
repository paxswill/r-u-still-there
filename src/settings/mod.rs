// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

mod cli;
pub(crate) mod gradient;
mod tracker;

use crate::camera::CameraSettings;
use crate::mqtt::MqttSettings;
use crate::render::RenderSettings;
use crate::stream::StreamSettings;
pub(crate) use cli::Args;
pub(crate) use tracker::TrackerSettings;

#[derive(Debug, Deserialize)]
pub(crate) struct Settings {
    /// A dummy field, present when parsing CLI arguments.
    #[serde(default)]
    config_path: Option<String>,

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

// DRY macro for merging in optional CLI arguments
macro_rules! merge_arg {
    ($arg:expr, $dest:expr) => {
        if let Some(arg_member) = $arg {
            $dest = arg_member;
        }
    };
}

impl Settings {
    /// Merge in values given on the command line into an existing [Settings].
    pub(crate) fn merge_args(&mut self, args: &Args) {
        if let Some(bus) = &args.i2c_bus {
            self.camera.kind.set_bus(bus.clone());
        }
        if let Some(address) = args.i2c_address {
            self.camera.kind.set_address(address);
        }
        if let Some(frame_rate) = args.frame_rate {
            self.camera.set_frame_rate(frame_rate)
        }
        merge_arg!(args.grid_size, self.render.grid_size);
        if let Some(units) = args.temperature_units {
            self.render.units = Some(units);
        }
        merge_arg!(args.colors, self.render.colors);
        merge_arg!(args.listen_address, self.streams.address);
        merge_arg!(args.listen_port, self.streams.port);
        // Only one of enable_mjpeg or disable_mjpeg can be true at a time, but it is is possible
        // for both to be false. In the case where both are false, the value read from defaults (or
        // a config file) is preserved.
        if args.enable_mjpeg {
            self.streams.mjpeg.enabled = true;
        } else if args.disable_mjpeg {
            self.streams.mjpeg.enabled = false;
        }
    }
}
