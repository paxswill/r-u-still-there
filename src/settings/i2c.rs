// SPDX-License-Identifier: GPL-3.0-or-later
use linux_embedded_hal::I2cdev;

use i2cdev::linux::LinuxI2CError;
use serde::Deserialize;

use std::convert::TryFrom;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Bus<'a> {
    Number(u8),
    Path(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct I2cSettings<'a> {
    //bus: &'a str,
    pub bus: Bus<'a>,
    pub address: u8,
}

impl<'a> TryFrom<I2cSettings<'a>> for I2cdev {
    type Error = LinuxI2CError;

    fn try_from(settings: I2cSettings) -> Result<Self, Self::Error> {
        let device_path = match settings.bus {
            Bus::Number(n) => PathBuf::from(&format!("/dev/i2c-{}", n)),
            Bus::Path(p) => PathBuf::from(p),
        };
        I2cdev::new(device_path)
    }
}
