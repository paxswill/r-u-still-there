// SPDX-License-Identifier: GPL-3.0-or-later
use std::sync::Arc;

use crate::image_buffer::ThermalImage;
use crate::temperature::Temperature;

#[derive(Clone, Debug)]
pub(crate) struct Measurement {
    pub(crate) image: Arc<ThermalImage>,
    pub(crate) temperature: Temperature,
}
