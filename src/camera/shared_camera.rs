// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{bail, Context};
use futures::stream::{StreamExt, TryStream};
use image::imageops;
use linux_embedded_hal::I2cdev;
use tokio::time::{self, Duration};
use tokio_stream::wrappers::IntervalStream;
use tracing::{info_span, warn};
use tracing_futures::Instrument;

use std::convert::{TryFrom, TryInto};
use std::sync::{Arc, Mutex};

use super::settings::{CameraKind, CameraSettings, Rotation};
use super::thermal_camera::{Mlx90640, ThermalCamera, YAxisDirection};
use crate::image_buffer::ThermalImage;
use crate::temperature::Temperature;

/// Types of cameras r-u-still-there is able to use.
///
/// Currently only GridEYEs are supported, but I hope to add others later on.
#[derive(Clone)]
pub(crate) struct Camera {
    camera: Arc<Mutex<dyn ThermalCamera + Send>>,
    settings: CameraSettings,
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
        let interval_millis = 1000 / self.settings.frame_rate() as u64;
        let interval = time::interval(Duration::from_millis(interval_millis));
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
        let camera: Arc<Mutex<dyn ThermalCamera + Send>> = match &settings.kind {
            CameraKind::GridEye(i2c) => {
                let bus = I2cdev::try_from(&i2c.bus).context("Unable to create I2C bus")?;
                let mut cam = amg88::GridEye::new(
                    bus,
                    i2c.address
                        .try_into()
                        .context("Invalid I2C address given")?,
                );
                match settings.frame_rate() {
                        n @ (1 | 10) => cam.set_frame_rate(n.try_into()?)?,
                        1..=9 => {
                            warn!("Polling the camera at a lower frame rate, but camera is still running at 10FPS");
                            cam.set_frame_rate(10.try_into()?)?
                        }
                        _ => bail!("Invalid GridEYE frame rate given. Only 1-10 are valid (and only 1 or 10 preferred)"),
                    }
                Arc::new(Mutex::new(cam))
            }
            CameraKind::Mlx909640(i2c) => {
                let bus = I2cdev::try_from(&i2c.bus).context("Unable to create I2C bus")?;
                let inner_camera = mlx9064x::Mlx90640Camera::new(bus, i2c.address)?;
                let mut camera_wrapper = Mlx90640::new(inner_camera);
                let frame_rate = settings.frame_rate();
                // TODO: Add support for 0.5 FPS
                let mlx_frame_rate = match frame_rate.cmp(&64) {
                    std::cmp::Ordering::Less | std::cmp::Ordering::Equal => {
                        if frame_rate.is_power_of_two() {
                            frame_rate
                        } else {
                            let actual_frame_rate = frame_rate.next_power_of_two();
                            warn!(
                                    "Polling the camera at a lower frame rate, but camera is still running at {}FPS",
                                    actual_frame_rate
                                );
                            actual_frame_rate
                        }
                    }
                    std::cmp::Ordering::Greater => {
                        bail!("Invalid MLX90640 frame rate given. Must be between 0 and 64 (powers of two preferred)")
                    }
                };
                camera_wrapper.set_frame_rate(mlx_frame_rate)?;
                Arc::new(Mutex::new(camera_wrapper))
            }
        };
        Ok(Self {
            camera,
            settings: settings.clone(),
        })
    }
}
