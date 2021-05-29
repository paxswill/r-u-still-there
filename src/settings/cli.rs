// SPDX-License-Identifier: GPL-3.0-or-later
use figment::value::{Dict, Map, Value};
use figment::{Error, Metadata, Profile, Provider};
use serde::{Deserialize, Serialize};
use structopt::clap::{AppSettings, ArgGroup, ArgMatches};
use structopt::StructOpt;

use std::net;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::str::FromStr;

use super::render::TemperatureUnit;
use crate::camera::Bus;

#[derive(Clone, Debug, Deserialize, Serialize, StructOpt)]
#[serde(rename_all = "lowercase")]
pub enum CameraKind {
    GridEye,
}

#[derive(Clone, Debug, StructOpt)]
#[structopt(setting(AppSettings::DeriveDisplayOrder), group = ArgGroup::with_name("mjpeg"))]
pub struct Args {
    /// Path to a configuration file.
    #[structopt(short, long, parse(from_os_str))]
    pub config_path: Option<PathBuf>,

    /// The kind of camera being used.
    #[structopt(short = "C", long, possible_values(&["grideye"]))]
    pub camera_kind: Option<CameraKind>,

    /// The I2C bus the camera is connected to.
    #[structopt(short = "b", long)]
    pub i2c_bus: Option<Bus>,

    /// The I2C address the camera is available at.
    #[structopt(short = "a", long, parse(try_from_str = parse_int_decimal_hex))]
    pub i2c_address: Option<u32>,

    /// The camera frame rate to use.
    #[structopt(short, long)]
    pub frame_rate: Option<u8>,

    /// The size of each camera pixel in the rendered image.
    #[structopt(short, long)]
    pub grid_size: Option<u8>,

    /// The unit to display the temperature in.
    #[structopt(short = "u", long = "units")]
    pub temperature_units: Option<TemperatureUnit>,

    /// The color scheme to use when rendering the thermal image.
    // TODO: better typing. Blocking on Gradient not having an easy way to get the name of the
    // gradient from the instance.
    //#[structopt(short = "o", long, parse(try_from_str = super::gradient::from_str))]
    #[structopt(short = "o", long)]
    pub colors: Option<String>,

    /// The IP address the streaming server should listen on.
    #[structopt(short = "l", long = "listen-address")]
    pub listen_address: Option<net::IpAddr>,

    /// The port number to bind the streaming server to.
    #[structopt(short = "p", long = "listen-port")]
    pub listen_port: Option<u16>,

    /// Enable MJPEG streaming.
    #[structopt(short = "m", long = "mjpeg", group = "mjpeg")]
    enable_mjpeg: bool,

    /// Disable MJPEG streaming.
    #[structopt(short = "M", long = "no-mjpeg", group = "mjpeg")]
    disable_mjpeg: bool,
}

#[derive(Clone, Debug)]
pub struct MatchedArgs<'a>(ArgMatches<'a>);

impl FromStr for CameraKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "grideye" => Ok(Self::GridEye),
            unknown => Err(format!("'{}' is not a known camera type", unknown)),
        }
    }
}

/// Parse an unsigned integer from a base-10 or base-16 string representation.
///
/// If the string starts with `0x`, the rest of the string is treated as a hexadecimal integer.
/// Otherwise the string is treated as a decimal integer.
#[allow(clippy::from_str_radix_10)]
fn parse_int_decimal_hex(num_str: &str) -> Result<u32, ParseIntError> {
    let num_str = num_str.to_ascii_lowercase();
    if let Some(hex_str) = num_str.strip_prefix("0x") {
        u32::from_str_radix(hex_str, 16)
    } else {
        u32::from_str_radix(num_str.as_str(), 10)
    }
}

impl Args {
    /// Create a map of camera-related settings.
    fn camera_data(&self) -> Result<Dict, Error> {
        let mut data = Dict::new();
        if let Some(camera_kind) = &self.camera_kind {
            data.insert("kind".to_string(), Value::serialize(camera_kind)?);
        }
        if let Some(i2c_bus) = &self.i2c_bus {
            data.insert("bus".to_string(), Value::serialize(i2c_bus)?);
        }
        if let Some(i2c_address) = self.i2c_address {
            data.insert("address".to_string(), Value::from(i2c_address));
        }
        if let Some(frame_rate) = self.frame_rate {
            data.insert("frame_rate".to_string(), Value::from(frame_rate));
        }
        Ok(data)
    }

    /// Create a map of render-related settings.
    fn render_data(&self) -> Result<Dict, Error> {
        let mut data = Dict::new();
        if let Some(grid_size) = self.grid_size {
            data.insert("grid_size".to_string(), Value::from(grid_size));
        }
        if let Some(temperature_units) = &self.temperature_units {
            data.insert("units".to_string(), Value::serialize(temperature_units)?);
        }
        if let Some(colors) = &self.colors {
            data.insert("colors".to_string(), Value::serialize(colors)?);
        }
        Ok(data)
    }

    /// Create a map of the streaming related settings.
    fn streams_data(&self) -> Result<Dict, Error> {
        let mut data = Dict::new();
        if let Some(listen_address) = &self.listen_address {
            data.insert("address".to_string(), Value::serialize(listen_address)?);
        }
        if let Some(listen_port) = self.listen_port {
            data.insert("port".to_string(), Value::from(listen_port));
        }
        // Only one of `enable_mjpeg` and `disable_mjpeg` can be true at a time, but it is possible
        // for *neither* of them to be true. In that case, don't include a value for the 'mjpeg'
        // key in the value map.
        if self.enable_mjpeg {
            data.insert("mjpeg".to_string(), Value::from(true));
        } else if self.disable_mjpeg {
            data.insert("mjpeg".to_string(), Value::from(false));
        }
        Ok(data)
    }
}

impl Provider for Args {
    fn metadata(&self) -> Metadata {
        Metadata::named("Command line arguments")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut data = Dict::new();
        if let Some(config_path) = &self.config_path {
            data.insert(
                "config_path".to_string(),
                Value::from(
                    config_path
                        .as_path()
                        .to_str()
                        .ok_or_else(|| "config file path isn't UTF-8".to_string())?,
                ),
            );
        }
        let camera_data = self.camera_data()?;
        if !camera_data.is_empty() {
            data.insert("camera".to_string(), Value::from(camera_data));
        }
        let render_data = self.render_data()?;
        if !render_data.is_empty() {
            data.insert("render".to_string(), Value::from(render_data));
        }
        let streams_data = self.streams_data()?;
        if !streams_data.is_empty() {
            data.insert("streams".to_string(), Value::from(streams_data));
        }
        Ok(Profile::Default.collect(data))
    }
}
