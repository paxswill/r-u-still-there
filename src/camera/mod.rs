// SPDX-License-Identifier: GPL-3.0-or-later
mod i2c;
mod settings;
mod shared_camera;
mod thermal_camera;

pub(crate) use i2c::{Bus, I2cSettings};
pub(crate) use settings::CameraSettings;
pub(crate) use shared_camera::Camera;
