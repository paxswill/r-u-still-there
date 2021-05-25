// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

use crate::occupancy::{Threshold, Tracker};

#[derive(Debug, Default, Deserialize)]
pub struct TrackerSettings {
    threshold: Threshold,
}

impl From<&TrackerSettings> for Tracker {
    fn from(settings: &TrackerSettings) -> Self {
        Self::new(settings.threshold.clone())
    }
}
