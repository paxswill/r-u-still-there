// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::Context;
use futures::stream::{StreamExt, TryStream};
use image::flat::{FlatSamples, SampleLayout};
use image::imageops;
use linux_embedded_hal::{i2cdev::linux::LinuxI2CError, I2cdev};
use serde_repr::Deserialize_repr;
use thermal_camera::{grideye, ThermalCamera};
use tokio::time::{self, Duration};
use tokio_stream::wrappers::IntervalStream;
use tracing::{debug, info_span};
use tracing_futures::Instrument;

use std::convert::{TryFrom, TryInto};
use std::sync::{Arc, Mutex};

use super::{i2c::I2cSettings, settings::CameraSettings};
use crate::image_buffer::ThermalImage;

/// Types of cameras r-u-still-there is able to use.
///
/// Currently only GridEYEs are supported, but I hope to add others later on.
#[derive(Clone)]
pub enum Camera {
    I2cCamera {
        camera: Arc<Mutex<dyn ThermalCamera<Error = LinuxI2CError>>>,
        settings: CommonSettings,
    },
}

/// Settings common to all camera types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommonSettings {
    pub rotation: Rotation,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub frame_rate: u8,
}

// This enum is purely used to restrict the acceptable values for rotation
#[derive(Clone, Copy, Deserialize_repr, PartialEq, Debug)]
#[repr(u16)]
pub enum Rotation {
    Zero = 0,
    Ninety = 90,
    OneEighty = 180,
    TwoSeventy = 270,
}

impl Camera {
    /// Retrieves a frame from the camera.
    ///
    /// The image has any transformations applied to it at this stage.
    pub fn get_frame(&self) -> anyhow::Result<ThermalImage> {
        // Only keep the lock as long as necessary
        let grid = {
            let camera = match self {
                Self::I2cCamera {
                    camera,
                    settings: _,
                } => camera,
            };
            let image = camera
                .lock()
                .expect("the camera lock to not be poisoned")
                .image();
            image
        }
        .context("Unable to retrieve thermal image")?;
        let (row_count, col_count) = grid.dim();
        let height = row_count as u32;
        let width = col_count as u32;
        // Force the layout to row-major. If it's already in that order, this is a noop
        // (and it *should* be in row-major order already).
        let grid = if grid.is_standard_layout() {
            grid
        } else {
            debug!("Reversing thermal image axes (not expected normally)");
            grid.reversed_axes()
        };
        let layout = SampleLayout::row_major_packed(1, width, height);
        let buffer_image = FlatSamples {
            samples: grid.into_raw_vec(),
            layout,
            color_hint: None,
        };
        // The provided ndarray is in standard layout, meaning all its data is contiguous.
        // The preconditions for try_into_buffer should all be met, so panic if there's a
        // problem.
        let mut temperatures = buffer_image
            .try_into_buffer()
            // try_into_buffer uses a 2-tuple as the error type, with the actual Error being the
            // first item in the tuple.
            .map_err(|e| e.0)
            .context("Unable to convert 2D array into an ImageBuffer")?;
        // ThermalCamera has the origin in the lower left, while image has it in the upper
        // left. Flip the image vertically by default to compensate for this, but skip the
        // flip if the user wants it flipped.
        if !self.common_settings().flip_vertical {
            imageops::flip_vertical_in_place(&mut temperatures);
        }
        // The rest of the basic image transformations
        if self.common_settings().flip_horizontal {
            imageops::flip_horizontal_in_place(&mut temperatures);
        }
        temperatures = match self.common_settings().rotation {
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
    pub fn get_temperature(&self) -> Option<f32> {
        match self {
            Self::I2cCamera {
                camera,
                settings: _,
            } => camera.lock().unwrap().temperature(),
        }
    }

    fn common_settings(&self) -> CommonSettings {
        match self {
            Self::I2cCamera {
                camera: _,
                settings,
            } => *settings,
        }
    }

    /// Create a [Stream] that yields a [ThermalImage] at the frame rate.
    //pub fn frame_stream(&self) -> impl Stream<Item = Result<ThermalImage, Arc<anyhow::Error>>> {
    pub fn frame_stream(
        &self,
    ) -> impl TryStream<Ok = ThermalImage, Error = anyhow::Error, Item = anyhow::Result<ThermalImage>>
    {
        let interval = time::interval(self.common_settings().into());
        // Really we just need a shared reference to self for get_frame()
        let this = self.clone();
        IntervalStream::new(interval)
            .map(
                move |_| this.get_frame(), //.map_err(|e| Arc::new(e)))
            )
            .instrument(info_span!("frame_stream"))
    }
}

impl TryFrom<&CameraSettings> for Camera {
    type Error = anyhow::Error;

    fn try_from(settings: &CameraSettings) -> anyhow::Result<Self> {
        let i2c_settings: &I2cSettings = settings.into();
        let bus = I2cdev::try_from(i2c_settings).context("Unable to create I2C bus")?;
        let camera = Arc::new(Mutex::new(match settings {
            CameraSettings::GridEye {
                i2c: i2c_settings,
                options: common_settings,
            } => {
                let mut cam = grideye::GridEye::new(
                    bus,
                    i2c_settings
                        .address
                        .try_into()
                        .context("Invalid I2C address given")?,
                );
                cam.set_frame_rate(
                    common_settings
                        .frame_rate
                        .try_into()
                        .context("Invalid frame rate given")?,
                )?;
                cam
            }
        }));
        let common_settings: CommonSettings = settings.clone().into();
        Ok(Self::I2cCamera {
            camera,
            settings: common_settings,
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
