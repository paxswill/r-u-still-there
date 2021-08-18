// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::TryFrom;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use image::flat::{FlatSamples, SampleLayout};
use linux_embedded_hal::I2cdev;
use tracing::debug;

use crate::image_buffer;
use crate::temperature::Temperature;

// The direction the Y-axis points in a thermal image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum YAxisDirection {
    Up,
    Down,
}

/// The combined output of a camera.
pub(crate) struct Measurement {
    /// The thermal image.
    ///
    /// The Y-axis may point either up or down.
    pub(super) image: image_buffer::ThermalImage,

    /// The direction the Y-axis points in the image.
    pub(super) y_direction: YAxisDirection,

    /// The temperature of the camera.
    pub(super) temperature: Temperature,

    /// How long to wait until the next frame is ready.
    pub(super) frame_delay: Duration,
}

/// The operations a thermal camera needs to implement to be used by r-u-still-there.
pub(crate) trait ThermalCamera {
    /// Take a measurement of the thermal data from the camera.
    ///
    /// A thermal image is captured, along with the camera temperature. The [`Duration`] to wait
    /// until the next measurement should be taken is also included in the returned value.
    fn measure(&mut self) -> anyhow::Result<Measurement>;

    /// Set the camera frame rate.
    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()>;
}

pub(crate) struct GridEye {
    camera: amg88::GridEye<I2cdev>,
    frame_rate: amg88::FrameRateValue,
}

impl GridEye {
    const DEFAULT_FRAME_RATE: amg88::FrameRateValue = amg88::FrameRateValue::Fps10;

    pub(crate) fn new(bus: I2cdev, address: amg88::Address) -> anyhow::Result<Self> {
        let mut camera = amg88::GridEye::new(bus, address);
        camera.set_frame_rate(Self::DEFAULT_FRAME_RATE)?;
        Ok(Self {
            camera,
            frame_rate: Self::DEFAULT_FRAME_RATE,
        })
    }
}

impl ThermalCamera for GridEye {
    fn measure(&mut self) -> anyhow::Result<Measurement> {
        let start = Instant::now();
        let temperature = self
            .camera
            .thermistor()
            .context("Error retrieving temperature from camera")
            .map(Temperature::Celsius)?;
        let grid = self.camera.image()?;
        let (row_count, col_count) = grid.dim();
        let height = row_count as u32;
        let width = col_count as u32;
        let layout = SampleLayout::row_major_packed(1, width, height);
        let buffer_image = FlatSamples {
            samples: grid.into_raw_vec(),
            layout,
            color_hint: None,
        };
        let image = buffer_image
            .try_into_buffer()
            // try_into_buffer uses a 2-tuple as the error type, with the actual Error being the
            // first item in the tuple.
            .map_err(|e| e.0)
            .context("Unable to convert 2D array into an ImageBuffer")?;
        let frame_duration = match self.frame_rate {
            amg88::FrameRateValue::Fps1 => Duration::from_secs(1),
            amg88::FrameRateValue::Fps10 => Duration::from_millis(100),
        };
        let frame_delay = frame_duration
            .checked_sub(start.elapsed())
            .unwrap_or(Duration::from_secs(0));
        Ok(Measurement {
            image,
            y_direction: YAxisDirection::Up,
            temperature,
            frame_delay,
        })
    }

    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()> {
        let grideye_frame_rate = amg88::FrameRateValue::try_from(frame_rate)
            .context("Invalid frame rate, only 1 or 10 are valid for GridEYE cameras")?;
        self.camera
            .set_frame_rate(grideye_frame_rate)
            .context("Error setting GridEYE frame rate")?;
        self.frame_rate = grideye_frame_rate;
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
pub(crate) struct $name {
    camera: $driver,

    /// The thermal image buffer.
    // It needs to be a Vec as different Melexis cameras have different resolutions.
    temperature_buffer: Vec<f32>,

    frame_duration: Option<Duration>,
}

impl $name
{
    pub(crate) fn new(camera: $driver) -> Self {
        let num_pixels = camera.height() * camera.width();
        Self {
            camera,
            temperature_buffer: vec![0f32; num_pixels],
            frame_duration: None,
        }
    }

    fn clone_thermal_image(&self) -> anyhow::Result<image_buffer::ThermalImage> {
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
        buffer_image
            .try_into_buffer()
            .map_err(|e| e.0)
            .context("Unable to convert ML9064x scratch buffer into an ImageBuffer")
    }
}

impl ThermalCamera for $name {
    fn measure(&mut self) -> anyhow::Result<Measurement> {
        // When frame_duration is None, read the current frame rate from the camera, then
        // synchronize with the camera
        let mut frame_duration = match self.frame_duration {
            Some(duration) => duration,
            None => {
                let full_frame_duration: Duration = self.camera.frame_rate()?.into();
                // Start with 20% faster than the frame rate, as that's what the datasheet
                // recommends. The actual frame rate will eventually be found by this function
                // anyways.
                let frame_duration = full_frame_duration.mul_f32(0.8);
                self.frame_duration = Some(frame_duration);
                self.camera.synchronize()?;
                frame_duration
            }
        };

        let start = Instant::now();
        // Get the thermal image first, as the temperature is retrieved and calculated as part of
        // that process.
        let mut num_data_checks: i32 = 0;
        let subpage = loop {
            num_data_checks += 1;
            if let Some(subpage) = self.camera.data_available()? {
                break subpage;
            }
            std::thread::yield_now();
        };
        let data_available_wait_duration = start.elapsed();
        self.camera.generate_image_subpage_to(subpage, &mut self.temperature_buffer)?;
        self.camera.reset_data_available()?;
        let image = self.clone_thermal_image()?;
        // Safe to unwrap as the temperature is calculated when the image is retrieved.
        let temperature = Temperature::Celsius(self.camera.ambient_temperature().unwrap());
        // Calculate how long to wait for. In a perfect world, this would be the per-frame
        // duration, less how long it takes to retrieve and calculate the thermal image and
        // temperature. In the real world, Melexis cameras run about 1-2% slow. The slow down is
        // (usually) consistent for a single frame rate, but is not consistent across camera
        // models. To correct for this, the frame duration is tracked within this wrapper struct,
        // and is adjusted so that the camera status register is checked twice for each frame
        // (checking twice so we know we're accessing a frame as soon as it's available). If fewer
        // checks are performed than expected, the frame rate is slowed down. If more checks that
        // expected are performed, the frame rate is sped up.
        let data_available_check_duration = data_available_wait_duration / num_data_checks as u32;
        let data_check_difference = num_data_checks - 2;
        if data_check_difference < 0 {
            // Not enough data checks. Only going to slow down by one at a time, as:
            // A) That's the only duration we have
            // B) We don't want to overshoot and go too slow
            // Also guard against transient spikes in latency by skipping the adjustment if it
            // is larger than the frame duration itself.
            if data_available_check_duration < frame_duration {
                frame_duration -= data_available_check_duration;
            } else {
                debug!(
                    "Skipping frame duration adjustment as {}us >= {}us",
                    data_available_check_duration.as_micros(),
                    frame_duration.as_micros()
                );
            }
        } else if data_check_difference > 0 {
            frame_duration += data_available_check_duration * data_check_difference as u32;
        }
        let measurement_duration = start.elapsed();
        let frame_delay = match frame_duration.checked_sub(measurement_duration) {
            None => {
                debug!(
                    ?measurement_duration,
                    ?frame_duration,
                    "Measurement time is greater than the frame duration"
                );
                // Just wait for the next frame, hopefully it will be better
                frame_duration
            }
            Some(frame_delay) => frame_delay,
        };
        self.frame_duration = Some(frame_duration);
        Ok(Measurement {
            image,
            y_direction: YAxisDirection::Down,
            temperature,
            frame_delay,
        })
    }

    fn set_frame_rate(&mut self, frame_rate: u8) -> anyhow::Result<()> {
        // TODO: Add a way to have <1 FPS frame rates
        let mlx_frame_rate = mlx9064x::FrameRate::try_from(frame_rate)
            .context("Invalid frame rate, only 1, 2, 4, 8, 16, 32, or 64 are valid for MLX9064* cameras")?;
        self.camera
            .set_frame_rate(mlx_frame_rate)
            .context("Error setting MLX9064x frame rate")?;
        // Let measure() know that a resync is needed
        self.frame_duration = None;
        Ok(())
    }
}
    };
}

melexis_camera!(Mlx90640, mlx9064x::Mlx90640Driver<I2cdev>);
melexis_camera!(Mlx90641, mlx9064x::Mlx90641Driver<I2cdev>);
