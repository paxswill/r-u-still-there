// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;
use structopt::clap::{AppSettings, ArgGroup};
use structopt::StructOpt;

use std::net;
use std::num::NonZeroU8;
use std::path::PathBuf;

use crate::camera::Bus;
use crate::mqtt::MqttUrl;
use crate::temperature::TemperatureUnit;
use crate::util::parse_int_decimal_hex;

#[derive(Clone, Debug, StructOpt)]
#[structopt(setting(AppSettings::DeriveDisplayOrder))]
#[structopt(group = ArgGroup::with_name("mjpeg"))]
#[structopt(group = ArgGroup::with_name("home_assistant"))]
pub(crate) struct Args {
    /// Path to a configuration file.
    #[structopt(short, long, parse(from_os_str))]
    pub(crate) config_path: Option<PathBuf>,

    /// The kind of camera being used.
    #[structopt(short = "C", long, possible_values(&["grideye"]))]
    pub(crate) camera_kind: Option<String>,

    /// The I2C bus the camera is connected to.
    #[structopt(short = "b", long)]
    pub(crate) i2c_bus: Option<Bus>,

    /// The I2C address the camera is available at.
    #[structopt(short = "a", long, parse(try_from_str = parse_int_decimal_hex))]
    pub(crate) i2c_address: Option<u8>,

    /// The camera frame rate to use.
    #[structopt(short, long)]
    pub(crate) frame_rate: Option<NonZeroU8>,

    /// The size of each camera pixel in the rendered image.
    #[structopt(short, long)]
    pub(crate) grid_size: Option<usize>,

    /// The unit to display the temperature in.
    #[structopt(short = "u", long = "units")]
    pub(crate) temperature_units: Option<TemperatureUnit>,

    /// The color scheme to use when rendering the thermal image.
    #[structopt(short = "o", long, parse(try_from_str = super::gradient::from_str))]
    pub(crate) colors: Option<colorous::Gradient>,

    /// The IP address the streaming server should listen on.
    #[structopt(short = "l", long = "listen-address")]
    pub(crate) listen_address: Option<net::IpAddr>,

    /// The port number to bind the streaming server to.
    #[structopt(short = "p", long = "listen-port")]
    pub(crate) listen_port: Option<u16>,

    /// Enable MJPEG streaming.
    #[structopt(short = "m", long = "mjpeg", group = "mjpeg")]
    pub(super) enable_mjpeg: bool,

    /// Disable MJPEG streaming.
    #[structopt(short = "M", long = "no-mjpeg", group = "mjpeg")]
    pub(super) disable_mjpeg: bool,

    /// The name for this device as exposed on the MQTT server.
    ///
    /// This name is used as part of the topics the sensor values are published to.
    #[structopt(short = "N", long)]
    pub(crate) mqtt_name: Option<String>,

    /// The (optional) username to used to connect to the MQTT broker.
    ///
    /// An empty string is interpreted as no username.
    #[structopt(short = "U", long)]
    pub(crate) mqtt_username: Option<String>,

    /// The (optional) password used to connect to the MQTT broker.
    ///
    /// An empty string is interpreted as no password.
    #[structopt(short = "P", long)]
    pub(crate) mqtt_password: Option<String>,

    /// The URL to the MQTT broker.
    ///
    /// The only schemes accepted are 'mqtt' and 'mqtts', with default ports of 1883 and 8883
    /// respectively.
    #[structopt(short = "S", long, parse(try_from_str))]
    pub(crate) mqtt_server: Option<MqttUrl>,

    /// Enable Home Assistant integration.
    #[structopt(long = "home-assistant", group = "home_assistant")]
    pub(super) enable_home_assistant: bool,

    /// Disable Home Assistant integration.
    #[structopt(long = "no-home-assistant", group = "home_assistant")]
    pub(super) disable_home_assistant: bool,
}
