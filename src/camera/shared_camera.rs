// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::Context;
use image::imageops;
use linux_embedded_hal::I2cdev;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, info, trace, warn};

#[cfg(feature = "mock_camera")]
use bincode::Options;

use std::convert::{TryFrom, TryInto};
use std::sync::{mpsc, Arc};
use std::thread::sleep as thread_sleep;

#[cfg(feature = "mock_camera")]
use std::io::{Read, Seek};

use super::measurement::Measurement;
use super::settings::{CameraKind, CameraSettings, Rotation};
use super::thermal_camera::{GridEye, Mlx90640, Mlx90641, ThermalCamera, YAxisDirection};

#[derive(Debug)]
pub(crate) enum CameraCommand {
    /// Create a new measurement listener and emit it on the response channel.
    Subscribe(oneshot::Sender<broadcast::Receiver<Measurement>>),
    /// Create a new command channel and return it on the provided channel.
    CreateCommandChannel(oneshot::Sender<mpsc::Sender<CameraCommand>>),
    /// Gracefully stop the camera thread.
    Shutdown,
}

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
}

impl Camera {
    /// Retrieve and publish measurements from the camera at the specified frame rate
    ///
    /// This is a blocking function that won't return until a [`CommandChannel::Shutdown`] is sent
    /// through a command channel from another thread (or it encounters an error).
    pub(crate) fn measurement_loop(mut self) -> anyhow::Result<()> {
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
            // Capture a measurement from the camera, apply image transformations, and wait for the
            // next frame.
            let super::thermal_camera::Measurement {
                mut image,
                y_direction,
                temperature,
                frame_delay,
            } = self.camera.measure()?;

            // If the image returned from the camera is with the Y-Axis pointing up, or if the
            // user has specified the image should be flipped, we need to flip the image it along
            // the Y-axis.
            if y_direction == YAxisDirection::Up || self.settings.flip_vertical {
                imageops::flip_vertical_in_place(&mut image);
            }
            // The rest of the basic image transformations
            if self.settings.flip_horizontal {
                imageops::flip_horizontal_in_place(&mut image);
            }
            image = match self.settings.rotation {
                Rotation::Zero => image,
                Rotation::Ninety => imageops::rotate90(&image),
                Rotation::OneEighty => {
                    imageops::rotate180_in_place(&mut image);
                    image
                }
                Rotation::TwoSeventy => imageops::rotate270(&image),
            };
            let channel_measurement = Measurement {
                image: Arc::new(image),
                temperature,
            };
            // Don't care if it fails or not, as failures are temporary.
            #[allow(unused_must_use)]
            {
                self.measurement_channel.send(channel_measurement);
            }
            trace!("Waiting {}us for the next frame", frame_delay.as_micros());
            thread_sleep(frame_delay);
        }
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
                let cam = GridEye::new(
                    bus,
                    i2c.address
                        .try_into()
                        .context("Invalid I2C address given")?,
                )?;
                Box::new(cam)
            }
            CameraKind::Mlx90640(i2c) => {
                let bus = I2cdev::try_from(&i2c.bus).context("Unable to create I2C bus")?;
                let inner_camera = mlx9064x::Mlx90640Driver::new(bus, i2c.address)?;
                let camera_wrapper = Mlx90640::new(inner_camera);
                Box::new(camera_wrapper)
            }
            CameraKind::Mlx90641(i2c) => {
                let bus = I2cdev::try_from(&i2c.bus).context("Unable to create I2C bus")?;
                let inner_camera = mlx9064x::Mlx90641Driver::new(bus, i2c.address)?;
                let camera_wrapper = Mlx90641::new(inner_camera);
                Box::new(camera_wrapper)
            }
            #[cfg(feature = "mock_camera")]
            CameraKind::MockCamera(data_path) => {
                let extension = data_path.extension().map(|s| s.to_str()).flatten();
                let measurements: Vec<super::MeasurementData> = match extension {
                    Some("toml") => {
                        let data_string = std::fs::read_to_string(data_path)?;
                        toml::from_str(&data_string).map_err(anyhow::Error::from)
                    }
                    // treat everything as bincode if we don't know to extension
                    _ => {
                        let mut measurements = Vec::new();
                        let mut file = std::fs::File::open(data_path)?;
                        let file_size = file.metadata()?.len();
                        // These are the options async-bincode uses (but skipping the limit).
                        let bincode_options = bincode::options()
                            .with_fixint_encoding()
                            .allow_trailing_bytes();
                        while file.stream_position()? < file_size {
                            // Have to keep cloning as bincode_options would otherwise be consumed
                            let frame = bincode_options.clone().deserialize_from(file.by_ref())?;
                            measurements.push(frame);
                        }
                        Ok(measurements)
                    }
                }?;
                let mock_cam =
                    super::mock_camera::MockCamera::new(measurements, super::RepeatMode::default());
                Box::new(mock_cam)
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
