// SPDX-License-Identifier: GPL-3.0-or-later
mod i2c;
mod settings;
mod shared_camera;

pub use i2c::{Bus, I2cSettings};
pub use settings::CameraSettings;
pub use shared_camera::{Camera, CommonSettings, Rotation};
