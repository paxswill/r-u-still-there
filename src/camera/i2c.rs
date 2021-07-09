// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::{Infallible, TryFrom};
use std::path::PathBuf;
use std::str::FromStr;

use i2cdev::linux::LinuxI2CError;
use linux_embedded_hal::I2cdev;
use serde::{Deserialize, Serialize};

use crate::util::parse_int_decimal_hex;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum Bus {
    Number(u32),
    Path(PathBuf),
}

impl Bus {
    pub(crate) fn path(&self) -> PathBuf {
        match self {
            Bus::Number(n) => PathBuf::from(format!("/dev/i2c-{}", n)),
            Bus::Path(p) => p.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct I2cSettings {
    pub(crate) bus: Bus,
    pub(crate) address: u8,
}

impl From<u32> for Bus {
    fn from(bus: u32) -> Self {
        Self::Number(bus)
    }
}

impl FromStr for Bus {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match parse_int_decimal_hex(s) {
            Ok(num) => Self::Number(num),
            Err(_) => Self::Path(PathBuf::from(s)),
        })
    }
}

impl TryFrom<&Bus> for I2cdev {
    type Error = LinuxI2CError;

    fn try_from(bus: &Bus) -> Result<Self, Self::Error> {
        I2cdev::new(bus.path())
    }
}

#[cfg(test)]
mod test {
    use super::Bus;
    use std::path::PathBuf;

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
        assert_eq!(bus, Bus::Path(PathBuf::from("/dev/i2c-0")));
    }

    #[test]
    fn bus_path() {
        let path_str = "/dev/i2c-9";
        let num_bus = Bus::Number(9);
        let path_bus = Bus::Path(PathBuf::from(path_str));
        let expected_path = PathBuf::from(path_str);
        assert_eq!(
            num_bus.path(),
            expected_path,
            "Numeric bus has an incorrect path"
        );
        assert_eq!(
            path_bus.path(),
            expected_path,
            "Path bus has an incorrect path"
        );
    }
}
