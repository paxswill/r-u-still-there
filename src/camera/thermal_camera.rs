// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::TryFrom;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use image::flat::{FlatSamples, SampleLayout};
use linux_embedded_hal::I2cdev;
use tracing::{debug, trace};

use crate::image_buffer;
use crate::moving_average::{BoxcarFilter, MovingAverage};
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
    fn set_frame_rate(&mut self, frame_rate: f32) -> anyhow::Result<()>;
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
            .unwrap_or_default();
        Ok(Measurement {
            image,
            y_direction: YAxisDirection::Up,
            temperature,
            frame_delay,
        })
    }

    fn set_frame_rate(&mut self, frame_rate: f32) -> anyhow::Result<()> {
        // Truncate the frame rate as the Grideye only has integer frame rates
        let frame_rate = frame_rate.trunc().clamp(0.0, u8::MAX as f32) as u8;
        let grideye_frame_rate = amg88::FrameRateValue::try_from(frame_rate)
            .context("Invalid frame rate, only 1 or 10 are valid for GridEYE cameras")?;
        self.camera
            .set_frame_rate(grideye_frame_rate)
            .context("Error setting GridEYE frame rate")?;
        self.frame_rate = grideye_frame_rate;
        Ok(())
    }
}

/// The result of polling for a new frame of data to be available from a Melexis camera.
#[derive(Copy, Clone, Debug, PartialEq)]
struct MelexisFramePoll {
    /// The point in time when the data became available.
    ///
    /// If there was more than one check made for data, we can be sure that we caught the frame as
    /// soon as it became available.
    frame_start: Option<Instant>,

    /// The number of checks for new data made.
    data_checks: i32,

    /// Which subpage is ready to be read from the camera.
    subpage: mlx9064x::Subpage,
}

const MELEXIS_MOVING_AVERAGE_LEN: usize = 10;

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

    previous_frame_start: Option<Instant>,

    average_frame_duration: BoxcarFilter<Duration, MELEXIS_MOVING_AVERAGE_LEN>,

    average_check_duration: BoxcarFilter<Duration, MELEXIS_MOVING_AVERAGE_LEN>,
}

impl $name {
    /// The number of poll durations to shorten the frame duration by while resynchronizing with
    /// the camera.
    const SHORTEN_POLL_COUNT: u32 = 2;

    /// The bounds for the number of checks to make.
    const NUM_CHECKS_BOUNDS: RangeInclusive<i32> = 3..=4;

    /// The weight for phase corrections. Applying the full phase correction usually ends up
    /// overshooting the target and then a resync is needed.
    const PHASE_GAIN: f32 = 0.75;

    pub(crate) fn new(camera: $driver) -> Self {
        let num_pixels = camera.height() * camera.width();
        Self {
            camera,
            temperature_buffer: vec![0f32; num_pixels],
            previous_frame_start: None,
            average_frame_duration: BoxcarFilter::new(),
            average_check_duration: BoxcarFilter::new(),
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

    fn poll_frame(&mut self) -> anyhow::Result<MelexisFramePoll> {
        let mut num_data_checks: i32 = 0;
        let subpage = loop {
            num_data_checks += 1;
            let data_available_start = Instant::now();
            let check_result = self.camera.data_available()?;
            let check_duration = data_available_start.elapsed();
            self.average_check_duration.push(check_duration);
            if let Some(subpage) = check_result {
                break subpage;
            }
            std::thread::yield_now();
        };
        let frame_start = if num_data_checks > 1 {
            Some(Instant::now())
        } else {
            None
        };
        Ok(MelexisFramePoll {
            frame_start,
            data_checks: num_data_checks,
            subpage,
        })
    }

    fn calculate_frame_delay(&mut self, poll_result: &MelexisFramePoll) -> Duration {
        // Safe to unwrap as self.poll_frame() populates average_check_duration
        let poll_duration = self.average_check_duration.current_value().unwrap();
        match (self.previous_frame_start, poll_result.frame_start) {
            (Some(prev_frame), Some(this_frame)) => {
                // Since we know both when this frame and the last frame started, we can assume the
                // frame rate is fairly close to the actual value. In this case there is no frame
                // length adjustment required.
                let frame_duration = self.average_frame_duration.update(this_frame.duration_since(prev_frame));
                // Only if we're synced up with the frame signal should phase corrections be
                // applied. We also know that the number of data checks for this frame was > 1, as
                // otherwise frame_start would be None.
                let phase_correction = if !Self::NUM_CHECKS_BOUNDS.contains(&poll_result.data_checks) {
                    poll_result.data_checks - Self::NUM_CHECKS_BOUNDS.end()
                } else {
                    0
                };
                trace!(
                    ?poll_result,
                    ?poll_duration,
                    ?frame_duration,
                    ?phase_correction,
                    "Calculating base frame duration from actual frames."
                );
                let phase_correction = phase_correction as f32 * Self::PHASE_GAIN;
                if phase_correction < 0f32 {
                    frame_duration - poll_duration.mul_f32(phase_correction.abs())
                } else if phase_correction > 0f32 {
                    frame_duration + poll_duration.mul_f32(phase_correction)
                } else {
                    frame_duration
                }
            }
            _ => {
                // If we're not synced with the frame signal, start shortening the current frame
                // rate until we get back in sync.
                // If we would end up with a negative duration, default to 0
                let current_duration = self.average_frame_duration
                    .current_value()
                    .and_then(|d| d.checked_sub(poll_duration * Self::SHORTEN_POLL_COUNT))
                    .unwrap_or_default();
                trace!(
                    ?poll_result,
                    ?poll_duration,
                    frame_duration = ?current_duration,
                    "Shortening current frame duration."
                );
                current_duration
            }
        }
    }
}

impl ThermalCamera for $name {

    fn measure(&mut self) -> anyhow::Result<Measurement> {
        let start = Instant::now();
        let poll_result = self.poll_frame()?;
        // Get the thermal image first, as the temperature is retrieved and calculated as part of
        // that process.
        self.camera.generate_image_subpage_to(poll_result.subpage, &mut self.temperature_buffer)?;
        self.camera.reset_data_available()?;
        let image = self.clone_thermal_image()?;
        // Safe to unwrap as the temperature is calculated when the image is retrieved.
        let temperature = Temperature::Celsius(self.camera.ambient_temperature().unwrap());
        // The basic frame delay, without compensating for how long the measurement took
        let base_frame_delay = self.calculate_frame_delay(&poll_result);
        // Update the frame start
        self.previous_frame_start = poll_result.frame_start;
        let measurement_duration = start.elapsed();
        let frame_delay = match base_frame_delay.checked_sub(measurement_duration) {
            None => {
                debug!(
                    ?measurement_duration,
                    ?base_frame_delay,
                    "Measurement time is greater than the frame duration"
                );
                // Just wait for the next frame, hopefully it will be better
                base_frame_delay
            }
            Some(frame_delay) => frame_delay,
        };
        Ok(Measurement {
            image,
            y_direction: YAxisDirection::Down,
            temperature,
            frame_delay,
        })
    }

    fn set_frame_rate(&mut self, frame_rate: f32) -> anyhow::Result<()> {
        let mlx_frame_rate = mlx9064x::FrameRate::try_from(frame_rate)
            .context("Invalid frame rate, only 0.5, 1, 2, 4, 8, 16, 32, or 64 are valid for MLX9064* cameras")?;
        self.camera
            .set_frame_rate(mlx_frame_rate)
            .context("Error setting MLX9064x frame rate")?;
        // Reset the frame duration average.
        self.average_frame_duration = BoxcarFilter::new();
        Ok(())
    }
}
    };
}

melexis_camera!(Mlx90640, mlx9064x::Mlx90640Driver<I2cdev>);
melexis_camera!(Mlx90641, mlx9064x::Mlx90641Driver<I2cdev>);
