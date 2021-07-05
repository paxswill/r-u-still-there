// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::Context;
use futures::stream::{StreamExt, TryStream};
use image::imageops;
use linux_embedded_hal::I2cdev;
use serde_repr::Deserialize_repr;
use tokio::time::{self, Duration};
use tokio_stream::wrappers::IntervalStream;
use tracing::info_span;
use tracing_futures::Instrument;

use std::convert::{TryFrom, TryInto};
use std::sync::{Arc, Mutex};

use super::thermal_camera::{ThermalCamera, YAxisDirection};
use super::{i2c::I2cSettings, settings::CameraSettings};
use crate::image_buffer::ThermalImage;
use crate::temperature::Temperature;

/// Types of cameras r-u-still-there is able to use.
///
/// Currently only GridEYEs are supported, but I hope to add others later on.
#[derive(Clone)]
pub(crate) struct Camera {
    camera: Arc<Mutex<dyn ThermalCamera + Send>>,
    settings: CommonSettings,
}

/// Settings common to all camera types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CommonSettings {
    pub(crate) rotation: Rotation,
    pub(crate) flip_horizontal: bool,
    pub(crate) flip_vertical: bool,
    pub(crate) frame_rate: u8,
}

// This enum is purely used to restrict the acceptable values for rotation
#[derive(Clone, Copy, Deserialize_repr, PartialEq, Debug)]
#[repr(u16)]
pub(crate) enum Rotation {
    Zero = 0,
    Ninety = 90,
    OneEighty = 180,
    TwoSeventy = 270,
}

impl Camera {
    /// Retrieves a frame from the camera.
    ///
    /// The image has any transformations applied to it at this stage.
    pub(crate) fn get_frame(&self) -> anyhow::Result<ThermalImage> {
        // Only keep the lock as long as necessary
        let (mut temperatures, y_axis) = {
            self.camera
                .lock()
                .expect("the camera lock to not be poisoned")
                .thermal_image()
        }
        .context("Unable to retrieve thermal image")?;
        // At the end of this function, the expected Y-axis direction is pointing down. If the
        // camera is returning an image different from that, or the user has said the image should
        // be flipped, we need to flip the image.
        // (writing this out to make sure I got the logic right)
        if y_axis == YAxisDirection::Up || self.settings.flip_vertical {
            imageops::flip_vertical_in_place(&mut temperatures);
        }
        // The rest of the basic image transformations
        if self.settings.flip_horizontal {
            imageops::flip_horizontal_in_place(&mut temperatures);
        }
        temperatures = match self.settings.rotation {
            Rotation::Zero => temperatures,
            Rotation::Ninety => imageops::rotate90(&temperatures),
            Rotation::OneEighty => {
                imageops::rotate180_in_place(&mut temperatures);
                temperatures
            }
            Rotation::TwoSeventy => imageops::rotate270(&temperatures),
        };
        Ok(temperatures)
    }

    /// Get the current temperature from the camera's thermal sensor (if it has one).
    pub(crate) fn get_temperature(&self) -> anyhow::Result<Temperature> {
        self.camera.lock().unwrap().temperature()
    }

    /// Create a [Stream] that yields a [ThermalImage] at the frame rate.
    pub(crate) fn frame_stream(
        &self,
    ) -> impl TryStream<Ok = ThermalImage, Error = anyhow::Error, Item = anyhow::Result<ThermalImage>>
    {
        let interval = time::interval(self.settings.into());
        // Really we just need a shared reference to self for get_frame()
        let this = self.clone();
        IntervalStream::new(interval)
            .map(move |_| this.get_frame())
            .instrument(info_span!("frame_stream"))
    }
}

impl TryFrom<&CameraSettings> for Camera {
    type Error = anyhow::Error;

    fn try_from(settings: &CameraSettings) -> anyhow::Result<Self> {
        let i2c_settings: &I2cSettings = settings.into();
        let bus = I2cdev::try_from(i2c_settings).context("Unable to create I2C bus")?;
        let camera: Arc<Mutex<dyn ThermalCamera + Send>> = Arc::new(Mutex::new(match settings {
            CameraSettings::GridEye { i2c, .. } => {
                let cam = amg88::GridEye::new(
                    bus,
                    i2c.address
                        .try_into()
                        .context("Invalid I2C address given")?,
                );
                cam
            }
        }));
        let common_settings = settings.common_settings();
        camera
            .lock()
            .unwrap()
            .set_frame_rate(common_settings.frame_rate)?;
        Ok(Self {
            camera,
            settings: *common_settings,
        })
    }
}

impl From<CommonSettings> for Duration {
    fn from(options: CommonSettings) -> Self {
        Self::from_millis(1000 / options.frame_rate as u64)
    }
}

impl Default for Rotation {
    fn default() -> Self {
        Self::Zero
    }
}
