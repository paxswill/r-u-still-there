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

/// DRY macro for merging in optional CLI arguments.
///
/// It handles three simple kinds of argument that occur in the [Args] struct. The first two handle
/// the case where a field needs to be checked if it is [Some], and if it is assign that value to a
/// field on a different struct. The difference in the two cases are that the first case handles
/// types that are [Copy], while the second handles [Clone]-able types.
///
///     merge_arg!(args.source.copyable, self.destination.field);
///     merge_arg!(clone args.source.clonable, self.destination.field);
///
/// The third case is for boolean values. These are implemented on [Args] as a pair of mutually
/// exclusive flags, meaning up to one may be true. `merge_args!` checks both flags, setting a
/// destination field `true` or `false` depending on which flag is set.
///
///     merge_arg!(args.enable_flag, args.disable_flag, self.is_enabled);
macro_rules! merge_arg {
    ($arg:expr, $dest:expr) => {
        if let Some(arg_member) = $arg {
            $dest = arg_member;
        }
    };
    (clone $arg:expr, $dest:expr) => {
        if let Some(arg_member) = &$arg {
            $dest = arg_member.clone();
        }
    };
    ($enable_arg:expr, $disable_arg:expr, $dest:expr) => {
        if $enable_arg {
            $dest = true;
        } else if $disable_arg {
            $dest = false;
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
        merge_arg!(
            args.enable_mjpeg,
            args.disable_mjpeg,
            self.streams.mjpeg.enabled
        );
        merge_arg!(clone args.mqtt_name, self.mqtt.name);
        // The MQTT username and password are allowed to be empty strings, in which case they are
        // interpreted as `None`
        match args.mqtt_username.as_deref() {
            Some("") => self.mqtt.username = None,
            Some(username) => self.mqtt.username = Some(username.to_string()),
            None => (),
        }
        match args.mqtt_password.as_deref() {
            Some("") => self.mqtt.password = None,
            Some(password) => self.mqtt.password = Some(password.into()),
            None => (),
        }
        merge_arg!(clone args.mqtt_server, self.mqtt.server);
        // Same situation for *_home_assistant as*_mjpeg.
        merge_arg!(
            args.enable_home_assistant,
            args.disable_home_assistant,
            self.mqtt.home_assistant.enabled
        );
    }
}
