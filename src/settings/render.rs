// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;
use structopt::StructOpt;

use crate::render::{Limit, TemperatureDisplay};
use crate::temperature::TemperatureUnit;

fn default_grid_size() -> usize {
    50
}

fn default_colors() -> colorous::Gradient {
    colorous::TURBO
}

impl From<Option<TemperatureUnit>> for TemperatureDisplay {
    fn from(optional_unit: Option<TemperatureUnit>) -> Self {
        match optional_unit {
            None => Self::Disabled,
            Some(unit) => TemperatureDisplay::Absolute(unit),
        }
    }
}

#[cfg(test)]
mod from_test {
    use super::TemperatureUnit;
    use crate::render::TemperatureDisplay;

    #[test]
    fn disabled() {
        assert_eq!(
            TemperatureDisplay::from(Option::<TemperatureUnit>::None),
            TemperatureDisplay::Disabled
        );
    }

    #[test]
    fn celsius() {
        assert_eq!(
            TemperatureDisplay::from(Some(TemperatureUnit::Celsius)),
            TemperatureDisplay::Absolute(TemperatureUnit::Celsius)
        );
    }

    #[test]
    fn fahrenheit() {
        assert_eq!(
            TemperatureDisplay::from(Some(TemperatureUnit::Fahrenheit)),
            TemperatureDisplay::Absolute(TemperatureUnit::Fahrenheit)
        );
    }
}

#[derive(Debug, Deserialize, StructOpt)]
pub struct RenderSettings {
    /// The size (in pixels) each camera pixel should be rendered as.
    #[structopt(short, long, default_value = "50")]
    #[serde(default = "default_grid_size")]
    pub grid_size: usize,

    #[structopt(short, long)]
    #[serde(default)]
    pub units: Option<TemperatureUnit>,

    #[structopt(skip)]
    #[serde(default)]
    pub upper_limit: Limit,

    #[structopt(skip)]
    #[serde(default)]
    pub lower_limit: Limit,

    #[structopt(short = "C", long, parse(try_from_str = super::gradient::from_str), default_value = "turbo")]
    #[serde(
        default = "default_colors",
        deserialize_with = "super::gradient::deserialize"
    )]
    pub colors: colorous::Gradient,
}

impl PartialEq for RenderSettings {
    fn eq(&self, other: &Self) -> bool {
        if self.grid_size != other.grid_size {
            return false;
        }
        if self.units != other.units {
            return false;
        }
        if self.upper_limit != other.upper_limit {
            return false;
        }
        if self.lower_limit != other.lower_limit {
            return false;
        }
        if format!("{:?}", self.colors) != format!("{:?}", other.colors) {
            return false;
        }
        true
    }
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            grid_size: default_grid_size(),
            units: None,
            upper_limit: Limit::default(),
            lower_limit: Limit::default(),
            colors: default_colors(),
        }
    }
}

#[cfg(test)]
mod render_test {
    use super::{RenderSettings, TemperatureUnit};
    use crate::render::Limit;

    #[test]
    fn defaults() {
        let parsed: Result<RenderSettings, _> = toml::from_str("");
        assert!(
            parsed.is_ok(),
            "Failed to parse empty TOML: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        let expected = RenderSettings::default();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn grid_size() {
        let parsed: Result<RenderSettings, _> = toml::from_str("grid_size = 42");
        assert!(
            parsed.is_ok(),
            "Failed to parse grid_size: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        let expected = RenderSettings {
            grid_size: 42,
            ..RenderSettings::default()
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn celsius() {
        let parsed: Result<RenderSettings, _> = toml::from_str("units = \"celsius\"");
        assert!(
            parsed.is_ok(),
            "Failed to parse celsius units: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        let expected = RenderSettings {
            units: Some(TemperatureUnit::Celsius),
            ..RenderSettings::default()
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn fahrenheit() {
        let parsed: Result<RenderSettings, _> = toml::from_str("units = \"fahrenheit\"");
        assert!(
            parsed.is_ok(),
            "Failed to parse fahrenheit units: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        let expected = RenderSettings {
            units: Some(TemperatureUnit::Fahrenheit),
            ..RenderSettings::default()
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn static_limit_inline() {
        let parsed: Result<RenderSettings, _> = toml::from_str("upper_limit = {\"static\" = 10}");
        assert!(
            parsed.is_ok(),
            "Failed to parse inline static limit: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        let expected = RenderSettings {
            upper_limit: Limit::Static(10f32),
            ..RenderSettings::default()
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    #[ignore]
    fn static_limit_dotted() {
        let parsed: Result<RenderSettings, _> = toml::from_str("upper_limit.static = 10");
        assert!(
            parsed.is_ok(),
            "Failed to parse dotted static limit: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        let expected = RenderSettings {
            upper_limit: Limit::Static(10f32),
            ..RenderSettings::default()
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn dynamic_limit() {
        let parsed: Result<RenderSettings, _> = toml::from_str("upper_limit = \"dynamic\"");
        assert!(
            parsed.is_ok(),
            "Failed to parse dotted static limit: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        let expected = RenderSettings {
            upper_limit: Limit::Dynamic,
            ..RenderSettings::default()
        };
        assert_eq!(parsed, expected);
    }
}
