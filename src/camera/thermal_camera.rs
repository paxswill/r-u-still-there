// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::TryFrom;
use std::error::Error as StdError;
use std::fmt;

use anyhow::Context as _;
use embedded_hal::blocking::i2c;
use image::flat::{FlatSamples, SampleLayout};
use tracing::{debug, warn};

use crate::image_buffer;
use crate::temperature::Temperature;

// The direction the Y-axis points in a thermal image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum YAxisDirection {
    Up,
    Down,
}

/// The operations a thermal camera needs to implement to be used by r-u-still-there.
pub(crate) trait ThermalCamera {
    /// Get the temperature of the camera.
    fn temperature(&mut self) -> anyhow::Result<Temperature>;

    /// Return a thermal image from a camera.
    fn thermal_image(&mut self) -> anyhow::Result<(image_buffer::ThermalImage, YAxisDirection)>;

    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()>;

    /// Block until a new image is available from the camera.
    ///
    /// Some camera modules do not synchronize access to their image data, which can result in
    /// corrupted image data. Not every camera module requires this; those that don't should
    /// implement it as a no-op.
    fn synchronize(&mut self) -> anyhow::Result<()>;
}

impl<I2C> ThermalCamera for amg88::GridEye<I2C>
where
    I2C: i2c::WriteRead,
    <I2C as i2c::WriteRead>::Error: 'static + StdError + Sync + Send,
{
    fn temperature(&mut self) -> anyhow::Result<Temperature> {
        self.thermistor()
            .context("Error retrieving temperature from camera")
            .map(Temperature::Celsius)
    }

    fn thermal_image(&mut self) -> anyhow::Result<(image_buffer::ThermalImage, YAxisDirection)> {
        let grid = self.image()?;
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
        let thermal_image = buffer_image
            .try_into_buffer()
            // try_into_buffer uses a 2-tuple as the error type, with the actual Error being the
            // first item in the tuple.
            .map_err(|e| e.0)
            .context("Unable to convert 2D array into an ImageBuffer")?;
        Ok((thermal_image, YAxisDirection::Up))
    }

    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()> {
        let grideye_frame_rate = amg88::FrameRateValue::try_from(frame_rate)
            .context("Invalid frame rate, only 1 or 10 are valid for GridEYE cameras")?;
        self.set_frame_rate(grideye_frame_rate)
            .context("Error setting GridEYE frame rate")
    }

    fn synchronize(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

// This is a dirty hack. I was having trouble implementing ThermalCamera while being generic over
// the underlying mlx9064x::CameraDriver. When GATs are stabilized, there's a 'gat' branch on
// mlx9064x and 'mlx9064x-gat' branch for r-u-still-there that are much simpler.
macro_rules! melexis_camera {
    ($name:ident, $driver:path) => {

/// A wrapper over Melexis cameras to implement [`ThermalCamera`]
///
/// Melexis cameras only update half of the frame at a time (either in a chessboard pattern or by
/// interleaving the rows). This wrapper uses an internal buffer so that it can provide the full
/// image at all times.
#[derive(Debug)]
pub(crate) struct $name<I2C> {
    camera: $driver,

    /// The thermal image buffer.
    // It needs to be a Vec as different Melexis cameras have different resolutions.
    temperature_buffer: Vec<f32>,
}

impl<I2C> $name<I2C>
where
    I2C: i2c::WriteRead + i2c::Write,
    <I2C as i2c::WriteRead>::Error: 'static + StdError + Sync + Send,
    <I2C as i2c::Write>::Error: 'static + StdError + Sync + Send,
{
    pub(crate) fn new(camera: $driver) -> Self {
        let num_pixels = camera.height() * camera.width();
        Self {
            camera,
            temperature_buffer: vec![0f32; num_pixels],
        }
    }
}

impl<I2C> ThermalCamera for $name<I2C>
where
    I2C: 'static + i2c::WriteRead + i2c::Write,
    <I2C as i2c::WriteRead>::Error: 'static + StdError + fmt::Debug + Sync + Send,
    <I2C as i2c::Write>::Error: 'static + StdError + fmt::Debug + Sync + Send,
{
    fn temperature(&mut self) -> anyhow::Result<Temperature> {
        let temperature = match self.camera.ambient_temperature() {
            Some(temperature) => temperature,
            None => {
                // Call get_image, and do it again!
                // TODO: Add quick ambient temperature calculation to mlx9064x so
                // ambient_temperature() can get rid of the Option.
                self.camera
                    .generate_image_to(&mut self.temperature_buffer)?;
                self.camera.ambient_temperature().unwrap()
            }
        };
        Ok(Temperature::Celsius(temperature))
    }

    fn thermal_image(&mut self) -> anyhow::Result<(image_buffer::ThermalImage, YAxisDirection)> {
        // TODO: There's something off in how the frames are being timed that I'm still tracking
        // down.
        if !self
            .camera
            .generate_image_if_ready(&mut self.temperature_buffer)?
        {
            warn!("Using stale data!");
        }
        // mlx9064x uses row-major ordering, so no swapping needed here.
        let layout = SampleLayout::row_major_packed(
            1,
            self.camera.width() as u32,
            self.camera.height() as u32,
        );
        let buffer_image = FlatSamples {
            // TODO: this clone could hurt performance. Investigate a shared container that then
            // keeps a single reference to the buffer.
            samples: self.temperature_buffer.clone(),
            layout,
            color_hint: None,
        };
        let thermal_image = buffer_image
            .try_into_buffer()
            .map_err(|e| e.0)
            .context("Unable to convert ML9064x scratch buffer into an ImageBuffer")?;
        Ok((thermal_image, YAxisDirection::Down))
    }

    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()> {
        // TODO: Add a way to have <1 FPS frame rates
        let mlx_frame_rate = mlx9064x::FrameRate::try_from(frame_rate)
            .context("Invalid frame rate, only 1, 2, 4, 8, 16, 32, or 64 are valid for MLX9064* cameras")?;
        self.camera
            .set_frame_rate(mlx_frame_rate)
            .context("Error setting MLX9064x frame rate")
    }

    fn synchronize(&mut self) -> anyhow::Result<()> {
        debug!("Synchronizing frame access");
        Ok(self.camera.synchronize()?)
    }
}
    };
}

melexis_camera!(Mlx90640, mlx9064x::Mlx90640Driver<I2C>);
melexis_camera!(Mlx90641, mlx9064x::Mlx90641Driver<I2C>);
