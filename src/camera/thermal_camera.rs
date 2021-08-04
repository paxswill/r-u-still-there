// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::TryFrom;
use std::error::Error as StdError;
use std::fmt;

use anyhow::Context as _;
use embedded_hal::blocking::i2c;
use image::flat::{FlatSamples, SampleLayout};
use ndarray::{ArrayViewMut, ShapeBuilder};
use tracing::debug;

use crate::image_buffer;
use crate::temperature::Temperature;

// The direction the Y-axis points in a thermal image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum YAxisDirection {
    Up,
    Down,
}

/// The operations a thermal camera needs to have to be used by r-u-still-there.
pub(crate) trait ThermalCamera {
    /// Get the temperature of the camera.
    fn temperature(&mut self) -> anyhow::Result<Temperature>;

    /// Return a thermal image from a camera.
    fn thermal_image(&mut self) -> anyhow::Result<(image_buffer::ThermalImage, YAxisDirection)>;

    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()>;
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
        let grideye_frame_rate =
            amg88::FrameRateValue::try_from(frame_rate).context("Invalid frame rate")?;
        self.set_frame_rate(grideye_frame_rate)
            .context("Error setting camera frame rate")
    }
}

#[derive(Debug)]
pub(crate) struct Mlx90640<I2C> {
    camera: mlx9064x::Mlx90640Driver<I2C>,
    temperature_buffer: Vec<f32>,
}

impl<I2C> Mlx90640<I2C>
where
    I2C: i2c::WriteRead + i2c::Write,
    <I2C as i2c::WriteRead>::Error: 'static + StdError + Sync + Send,
    <I2C as i2c::Write>::Error: 'static + StdError + Sync + Send,
{
    pub(crate) fn new(camera: mlx9064x::Mlx90640Driver<I2C>) -> Self {
        let num_pixels = camera.height() * camera.width();
        Self {
            camera,
            temperature_buffer: vec![0f32; num_pixels],
        }
    }
}

impl<I2C> ThermalCamera for Mlx90640<I2C>
where
    I2C: 'static + i2c::WriteRead + i2c::Write,
    <I2C as i2c::WriteRead>::Error: 'static + StdError + Sync + Send,
    <I2C as i2c::Write>::Error: 'static + StdError + Sync + Send,
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
        self.camera
            .generate_image_if_ready(&mut self.temperature_buffer)?;
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
            .context("Unable to convert ML90640 scratch buffer into an ImageBuffer")?;
        Ok((thermal_image, YAxisDirection::Down))
    }

    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()> {
        // TODO: Add a way to have <1 FPS frame rates
        let mlx_frame_rate =
            mlx9064x::FrameRate::try_from(frame_rate).context("Invalid frame rate")?;
        self.camera
            .set_frame_rate(mlx_frame_rate)
            .context("Error setting camera frame rate")
    }
}
