// SPDX-License-Identifier: GPL-3.0-or-later
use image::imageops;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, info, trace, warn};

use std::convert::TryFrom;
use std::sync::{mpsc, Arc};
use std::thread::sleep as thread_sleep;

use super::measurement::Measurement;
use super::settings::{CameraSettings, Rotation};
use super::thermal_camera::{ThermalCamera, YAxisDirection};

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
    rotation: Rotation,
    flip_vertical: bool,
    flip_horizontal: bool,
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
            if y_direction == YAxisDirection::Up || self.flip_vertical {
                imageops::flip_vertical_in_place(&mut image);
            }
            // The rest of the basic image transformations
            if self.flip_horizontal {
                imageops::flip_horizontal_in_place(&mut image);
            }
            image = match self.rotation {
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

    fn try_from(settings: &CameraSettings) -> Result<Self, Self::Error> {
        let mut camera = settings.create_camera()?;
        camera.set_frame_rate(settings.frame_rate())?;
        let (measurement_channel, _) = broadcast::channel(1);
        let (command_sender, command_receiver) = mpsc::channel();
        Ok(Self {
            camera,
            rotation: settings.rotation(),
            flip_vertical: settings.flip_vertical(),
            flip_horizontal: settings.flip_horizontal(),
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
