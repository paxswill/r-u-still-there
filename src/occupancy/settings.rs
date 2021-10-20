// SPDX-License-Identifier: GPL-3.0-or-later
use std::time::Duration;

use serde::Deserialize;
use serde_with::serde_as;

use super::gmm::GmmParameters;

/// Settings for the people tracker.
#[serde_as]
#[derive(Copy, Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct TrackerSettings {
    /// Background subtraction settings.
    ///
    /// The defaults are usually sufficient for most use cases.
    #[serde(default)]
    pub(crate) background_model_parameters: GmmParameters,

    /// Background confidence threshold.
    #[serde(default = "TrackerSettings::default_confidence_threshold")]
    pub(crate) background_confidence_threshold: f32,

    /// The minimum size for an object to be considered a person.
    #[serde(default)]
    pub(crate) minimum_size: Option<usize>,

    /// How long before a stationary object is ignored.
    ///
    /// Whenever an object moves, its stationary timeout is reset. After *stationary_timeout*
    /// seconds of not moving, an object that was previously marked as a person is no longer
    /// considered on (until they move again).
    #[serde_as(as = "serde_with::DurationSeconds<u64>")]
    #[serde(default = "TrackerSettings::default_stationary_timeout")]
    pub(crate) stationary_timeout: Duration,
}

impl TrackerSettings {
    const fn default_confidence_threshold() -> f32 {
        0.001
    }

    const fn default_stationary_timeout() -> Duration {
        // Three hour stationary timeout
        Duration::from_secs(60 * 60 * 3)
    }
}

impl Default for TrackerSettings {
    fn default() -> Self {
        Self {
            background_model_parameters: GmmParameters::default(),
            background_confidence_threshold: Self::default_confidence_threshold(),
            minimum_size: None,
            stationary_timeout: Self::default_stationary_timeout(),
        }
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use super::{GmmParameters, TrackerSettings};

    #[test]
    fn defaults() -> anyhow::Result<()> {
        let source = "";
        let config: TrackerSettings = toml::from_str(source)?;
        let expected = TrackerSettings {
            background_model_parameters: GmmParameters::default(),
            background_confidence_threshold: TrackerSettings::default_confidence_threshold(),
            minimum_size: None,
            stationary_timeout: TrackerSettings::default_stationary_timeout(),
        };
        assert_eq!(config, expected);
        Ok(())
    }

    #[test]
    fn minimum_size() -> anyhow::Result<()> {
        let source = r#"
        minimum_size = 3
        "#;
        let config: TrackerSettings = toml::from_str(source)?;
        let expected = TrackerSettings {
            minimum_size: Some(3),
            ..Default::default()
        };
        assert_eq!(config, expected);
        Ok(())
    }

    #[test]
    fn timeout_seconds() -> anyhow::Result<()> {
        let source = r#"
        stationary_timeout = 3600
        "#;
        let config: TrackerSettings = toml::from_str(source)?;
        let expected = TrackerSettings {
            stationary_timeout: Duration::from_secs(3600),
            ..Default::default()
        };
        assert_eq!(config, expected);
        Ok(())
    }
}
