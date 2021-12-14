// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;
use tracing::debug;

#[derive(Copy, Clone, Debug, PartialEq, Deserialize)]
#[serde(from = "f32")]
pub(crate) enum LearningRate {
    Initializing {
        sample_count: i32,
        target_value: f32,
    },
    Trained(f32),
}

impl LearningRate {
    pub(super) const fn new(target_value: f32) -> Self {
        Self::Initializing {
            sample_count: 1,
            target_value,
        }
    }

    pub(super) fn increment(&mut self) {
        let target_value = match self {
            LearningRate::Initializing {
                sample_count,
                target_value,
            } => {
                *sample_count += 1;
                *target_value
            }
            LearningRate::Trained(_) => {
                return;
            }
        };
        if self.current_value() <= target_value {
            debug!(
                ?target_value,
                "Initialization period complete, target learning rate reached"
            );
            *self = Self::Trained(target_value);
        }
    }

    pub(super) fn current_value(&self) -> f32 {
        match self {
            LearningRate::Initializing { sample_count, .. } => (*sample_count as f32).recip(),
            LearningRate::Trained(value) => *value,
        }
    }

    pub(super) const fn is_trained(&self) -> bool {
        matches!(self, Self::Trained(..))
    }
}

impl From<f32> for LearningRate {
    fn from(target: f32) -> Self {
        Self::new(target)
    }
}
