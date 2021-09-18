// SPDX-License-Identifier: GPL-3.0-or-later
use std::time::Duration;

use serde::Deserialize;

use crate::occupancy::{Threshold, Tracker};

/// A newtype wrapper for stationary timeouts in seconds.
#[derive(Copy, Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Timeout(u64);

impl From<Duration> for Timeout {
    fn from(duration: Duration) -> Self {
        Self(duration.as_secs())
    }
}

impl From<Timeout> for Duration {
    fn from(timeout: Timeout) -> Self {
        Duration::from_secs(timeout.0)
    }
}


impl Default for Timeout {
    // Default timeout is 3 hours
    fn default() -> Self {
        Self(60 * 60 * 3)
    }
}

#[derive(Debug, Default, Deserialize, PartialEq)]
pub(crate) struct TrackerSettings {
    /// The threshold temperature for whether a person occupies a pixel.
    #[serde(default)]
    pub(crate) threshold: Threshold,

    /// The minimum size for an object to be considered a person.
    #[serde(default)]
    pub(crate) minimum_size: Option<u32>,

    /// How long before a stationary object is ignored.
    ///
    /// Whenever an object moves, it's stationary timeout is reset. After *stationary_timeout*
    /// seconds of not moving, an object that was previously marked as a person is no longer
    /// considered on (until they move again).
    #[serde(default)]
    pub(crate) stationary_timeout: Timeout,
}

impl From<&TrackerSettings> for Tracker {
    fn from(settings: &TrackerSettings) -> Self {
        Self::new(settings.threshold.clone())
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use crate::occupancy::Threshold;
    use crate::temperature::Temperature;

    use super::{Timeout, TrackerSettings};

    #[test]
    fn duration_to_timeout() {
        const SECONDS: u64 = 75;
        assert_eq!(
            Timeout::from(Duration::from_secs(SECONDS)),
            Timeout(SECONDS)
        )
    }

    #[test]
    fn timeout_to_duration() {
        const SECONDS: u64 = 45;
        assert_eq!(
            Duration::from(Timeout(SECONDS)),
            Duration::from_secs(SECONDS)
        )
    }

    #[test]
    fn defaults() -> anyhow::Result<()> {
        let source = "";
        let config: TrackerSettings = toml::from_str(source)?;
        let expected = TrackerSettings {
            threshold: Threshold::Automatic,
            minimum_size: None,
            stationary_timeout: Duration::from_secs(3600 * 3).into(),
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
            ..Default::default()
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
            stationary_timeout: Timeout(3600),
            ..Default::default()
        };
        assert_eq!(config, expected);
        Ok(())
    }
}
