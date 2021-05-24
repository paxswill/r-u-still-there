// SPDX-License-Identifier: GPL-3.0-or-later
use linux_embedded_hal::I2cdev;

use i2cdev::linux::LinuxI2CError;
use serde::{Deserialize, Serialize};

use std::convert::TryFrom;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Bus {
    Number(u32),
    Path(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct I2cSettings {
    pub bus: Bus,
    pub address: u8,
}

impl From<u32> for Bus {
    fn from(bus: u32) -> Self {
        Self::Number(bus)
    }
}

impl FromStr for Bus {
    type Err = serde_yaml::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_yaml::from_str(s)
    }
}

impl TryFrom<&I2cSettings> for I2cdev {
    type Error = LinuxI2CError;

    fn try_from(settings: &I2cSettings) -> Result<Self, Self::Error> {
        let device_path = match &settings.bus {
            Bus::Number(n) => PathBuf::from(format!("/dev/i2c-{}", n)),
            Bus::Path(p) => PathBuf::from(p),
        };
        I2cdev::new(device_path)
    }
}

#[cfg(test)]
mod test {
    use super::Bus;

    #[test]
    fn bus_from_num() {
        assert_eq!(Bus::from(0), Bus::Number(0))
    }

    #[test]
    fn bus_num_from_decimal_string() {
        let bus: Result<Bus, _> = "0".parse();
        assert!(bus.is_ok());
        let bus = bus.unwrap();
        assert_eq!(bus, Bus::Number(0))
    }

    #[test]
    fn bus_num_from_hex_string() {
        let bus: Result<Bus, _> = "0x68".parse();
        assert!(bus.is_ok());
        let bus = bus.unwrap();
        assert_eq!(bus, Bus::Number(0x68))
    }

    #[test]
    fn bus_path_from_string() {
        let bus: Result<Bus, _> = "/dev/i2c-0".parse();
        assert!(bus.is_ok());
        let bus = bus.unwrap();
        assert_eq!(bus, Bus::Path("/dev/i2c-0".to_string()))
    }
}
