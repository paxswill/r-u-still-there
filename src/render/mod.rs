// SPDX-License-Identifier: GPL-3.0-or-later

use crate::temperature::TemperatureUnit;

pub(crate) mod color;
pub(crate) mod color_map;
pub(crate) mod font;
pub(crate) mod layer;
mod resize;
mod settings;
pub(crate) use settings::RenderSettings;

#[cfg(feature = "render_fontdue")]
mod cheese;
#[cfg(feature = "render_svg")]
mod svg;

/// Control how the temperature of each pixel is displayed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TemperatureDisplay {
    /// Don't show the temperature.
    Disabled,

    /// Display the absolute temperature in the given units.
    Absolute(TemperatureUnit),
}

impl Default for TemperatureDisplay {
    fn default() -> Self {
        Self::Disabled
    }
}
