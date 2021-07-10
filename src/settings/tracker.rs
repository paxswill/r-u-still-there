// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

use crate::occupancy::{Threshold, Tracker};

#[derive(Debug, Default, Deserialize, PartialEq)]
pub(crate) struct TrackerSettings {
    #[serde(default)]
    pub(crate) threshold: Threshold,
}

impl From<&TrackerSettings> for Tracker {
    fn from(settings: &TrackerSettings) -> Self {
        Self::new(settings.threshold.clone())
    }
}

#[cfg(test)]
mod test {
    use crate::occupancy::Threshold;
    use crate::temperature::Temperature;

    use super::TrackerSettings;

    #[test]
    fn automatic() -> anyhow::Result<()> {
        let source = "";
        let config: TrackerSettings = toml::from_str(source)?;
        let expected = TrackerSettings {
            threshold: Threshold::Automatic,
        };
        assert_eq!(config, expected);
        Ok(())
    }

    // Parsing of temperature values is tested in the temperature module.
    #[test]
    fn static_threshold() -> anyhow::Result<()> {
        let source = r#"
        threshold = 7
        "#;
        let config: TrackerSettings = toml::from_str(source)?;
        let expected = TrackerSettings {
            threshold: Threshold::Static(Temperature::Celsius(7f32)),
        };
        assert_eq!(config, expected);
        Ok(())
    }
}
