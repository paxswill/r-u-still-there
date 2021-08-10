// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{bail, Context};
use image::imageops;
use linux_embedded_hal::I2cdev;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, error, info, warn};

use std::convert::{TryFrom, TryInto};
use std::sync::{mpsc, Arc};
use std::thread::{sleep as thread_sleep, Builder, JoinHandle as ThreadJoinHandle};
use std::time::{Duration, Instant};

use super::settings::{CameraKind, CameraSettings, Rotation};
use super::thermal_camera::{Mlx90640, ThermalCamera, YAxisDirection};
use crate::image_buffer::ThermalImage;
use crate::temperature::Temperature;

#[derive(Clone, Debug)]
pub(crate) struct Measurement {
    pub(crate) image: Arc<ThermalImage>,
    pub(crate) temperature: Temperature,
}

#[derive(Debug)]
pub(crate) enum CameraCommand {
    /// Create a new measurement listener and emit it on the response channel.
    Subscribe(oneshot::Sender<broadcast::Receiver<Measurement>>),
    /// Create a new command channel and return it on the provided channel.
    CreateCommandChannel(oneshot::Sender<mpsc::Sender<CameraCommand>>),
    /// Gracefully stop the camera thread.
    Shutdown,
}

const MICROS_IN_SECOND: u64 = 1_000_000;

/// How many frames to process before synchronizing camera frame access.
const FRAME_RESYNC_PERIOD: u32 = 1_000_000;

/// Retrieve measurements from a camera.
///
/// This structure runs on a separate thread in an attempt to keep the timing as close to the
/// camera frame rate as possible.
pub(crate) struct Camera {
    camera: Box<dyn ThermalCamera + Send>,
    settings: CameraSettings,
    measurement_channel: broadcast::Sender<Measurement>,
    command_receiver: mpsc::Receiver<CameraCommand>,
    command_sender: mpsc::Sender<CameraCommand>,
    resync_counter: u32,
}

impl Camera {
    /// Spawn the camera thread.
    pub(crate) fn spawn(mut self) -> std::io::Result<ThreadJoinHandle<anyhow::Result<()>>> {
        Builder::new()
            .name("camera frame access".to_string())
            .spawn(move || {
                // Log the error, so if the join handle isn't checked before termination we still
                // get logs out of it
                let res = self.measurement_loop();
                if let Err(ref error) = res {
                    error!("Camera loop error: {:?}", error)
                }
                res
            })
    }

    /// Retrieve and publish measurements from the camera at the specified frame rate
    ///
    /// This method is meant to be called from a separate thread. It will only return on error, or
    /// if a [`CameraCommand::Shutdown`] is sent on the command channel.
    fn measurement_loop(&mut self) -> anyhow::Result<()> {
        let frame_duration =
            Duration::from_micros(MICROS_IN_SECOND / self.settings.frame_rate() as u64);
        loop {
            // Respond to any pending commands
            for cmd in self.command_receiver.try_iter() {
                match cmd {
                    CameraCommand::Subscribe(response) => {
                        debug!("Camera loop: replying to new subscriber");
                        warn_on_oneshot_error(response.send(self.measurement_channel.subscribe()))
                    }
                    CameraCommand::CreateCommandChannel(response) => {
                        debug!("Camera loop: creating new command channel");
                        warn_on_oneshot_error(response.send(self.command_sender.clone()))
                    }
                    CameraCommand::Shutdown => {
                        info!("Terminating camera loop");
                        return Ok(());
                    }
                }
            }
            // Periodically synchronize frame access
            if self.resync_counter == 0 {
                self.camera.synchronize()?;
            } else if self.resync_counter >= FRAME_RESYNC_PERIOD {
                self.resync_counter = 0;
            } else {
                self.resync_counter += 1;
            }
            // Capture an image and measure the temperature then send it off to any subscribers.
            let start_processing = Instant::now();
            let image = Arc::new(self.get_frame()?);
            let temperature = self.get_temperature()?;
            let measurement = Measurement { image, temperature };
            // Don't care if it fails or not, as failures are temporary.
            #[allow(unused_must_use)]
            {
                self.measurement_channel.send(measurement);
            }
            // Sleep until the next frame
            let processing_time = start_processing.elapsed();
            if processing_time < frame_duration {
                let until_next_frame = frame_duration - processing_time;
                thread_sleep(until_next_frame)
            }
        }
    }

    /// Retrieves a frame from the camera.
    ///
    /// The image has any transformations applied to it at this stage.
    fn get_frame(&mut self) -> anyhow::Result<ThermalImage> {
        let (mut temperatures, y_axis) = self
            .camera
            .thermal_image()
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
    fn get_temperature(&mut self) -> anyhow::Result<Temperature> {
        self.camera.temperature()
    }

    pub(crate) fn command_channel(&self) -> mpsc::Sender<CameraCommand> {
        self.command_sender.clone()
    }
}

impl TryFrom<&CameraSettings> for Camera {
    type Error = anyhow::Error;

    fn try_from(settings: &CameraSettings) -> anyhow::Result<Self> {
        let mut camera: Box<dyn ThermalCamera + Send> = match &settings.kind {
            CameraKind::GridEye(i2c) => {
                let bus = I2cdev::try_from(&i2c.bus).context("Unable to create I2C bus")?;
                let cam = amg88::GridEye::new(
                    bus,
                    i2c.address
                        .try_into()
                        .context("Invalid I2C address given")?,
                );
                Box::new(cam)
            }
            CameraKind::Mlx90640(i2c) => {
                let bus = I2cdev::try_from(&i2c.bus).context("Unable to create I2C bus")?;
                let inner_camera = mlx9064x::Mlx90640Driver::new(bus, i2c.address)?;
                let camera_wrapper = Mlx90640::new(inner_camera);
                Box::new(camera_wrapper)
            }
        };
        camera.set_frame_rate(settings.frame_rate())?;
        let (measurement_channel, _) = broadcast::channel(1);
        let (command_sender, command_receiver) = mpsc::channel();
        Ok(Self {
            camera,
            settings: settings.clone(),
            measurement_channel,
            command_receiver,
            command_sender,
            resync_counter: 0,
        })
    }
}

/// Simply a warning function for oneshot::Sender::send() errors.
fn warn_on_oneshot_error<T>(oneshot_send_result: Result<(), T>) {
    match oneshot_send_result {
        Ok(_) => (),
        Err(_) => {
            // FIXME: This is an incredibly obtuse message
            warn!("Camera task command response receiver hung up early");
        }
    }
}
