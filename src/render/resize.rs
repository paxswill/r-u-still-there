// SPDX-License-Identifier: GPL-3.0-or-later
use std::convert::TryFrom;
use std::error::Error as StdError;
use std::fmt;

use async_trait::async_trait;
use image::RgbaImage;
use serde::Deserialize;

use super::settings::RenderSettings;

/// Different resizing methods

#[derive(Copy, Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Method {
    /// Nearest neighbor sampling.
    Nearest,

    /// Triangle (aka linear) sampling.
    #[serde(alias = "linear")]
    Triangle,

    /// Catmull-Rom (aka bicubic) sampling.
    #[serde(alias = "bicubic")]
    CatmullRom,

    /// Mitchell-Netravali sampling.
    Mitchell,

    /// Lanczos sampling with a window size of 3.
    #[serde(alias = "lanczos")]
    Lanczos3,
}

impl Default for Method {
    fn default() -> Self {
        Self::Nearest
    }
}

#[derive(Debug)]
pub(crate) enum ResizeError {
    /// When a resizer does not support the requested method.
    UnsupportedMethod,

    /// Other kinds of errors.
    Other(anyhow::Error),
}

impl fmt::Display for ResizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResizeError::UnsupportedMethod => f.write_str("Unsupported resize method given"),
            ResizeError::Other(err) => err.fmt(f),
        }
    }
}

impl StdError for ResizeError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            ResizeError::UnsupportedMethod => None,
            ResizeError::Other(err) => Some(err.as_ref()),
        }
    }
}

#[async_trait]
pub(crate) trait Resizer: fmt::Debug {
    /// Enlarge a map of colors.
    ///
    /// The [`RenderSettings.grid_size`] is the scaling factor, and
    /// [`RenderSettings.scaling_method`] specifies the scaling method.
    /// [`ResizeError::UnsupportedMethod`].
    async fn enlarge(&self, colors: RgbaImage) -> RgbaImage;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PointResize(u32);

impl<'a> TryFrom<&'a RenderSettings> for PointResize {
    type Error = ResizeError;

    fn try_from(settings: &'a RenderSettings) -> Result<Self, Self::Error> {
        match settings.scaling_method {
            Method::Nearest => Ok(Self(settings.grid_size as u32)),
            _ => Err(ResizeError::UnsupportedMethod),
        }
    }
}

#[async_trait]
impl Resizer for PointResize {
    /// This is a fast way to enlarge a grid of individual pixels. Each input pixel will be
    /// enlarged to a `grid_size` square.
    ///
    /// The current implementation builds a series of mono-color image views (using
    /// [image::flat::FlatSamples::with_monocolor]), then drawing these grid squares on to the
    /// final image using [image::imageops::replace]. Alternative implementations that were tested
    /// include:
    ///
    /// * [image::imageops::resize] with [nearest neighbor][image::imageops::FilterType::Nearest]
    ///   filtering. This seems to increase runtime exponentially; with a 30 pixel grid size, a
    ///   BeagleBone Black/Green could (barely) keep up with a 10 FPS GridEYE image, but at 50
    ///   pixels would lag to roughly 2 FPS.
    /// * Duplicating individual pixels using `flat_map`, `repeat` and `take`, then `collect`ing
    ///   everything into a vector. This was faster than `resize`, but still not fast enough.
    /// * As above, but pre-allocating the vector. No significant change.
    ///
    /// With this implementation, a BeagleBone Black/Green can server up an MJPEG stream with 50
    /// pixel grid squares at 10 FPS while keeping CPU usage below 50%.

    async fn enlarge(&self, colors: RgbaImage) -> RgbaImage {
        let grid_size = self.0;
        let mut full_image =
            image::ImageBuffer::new(colors.width() * grid_size, colors.height() * grid_size);
        for (x, y, pixel) in colors.enumerate_pixels() {
            let tile = image::flat::FlatSamples::with_monocolor(pixel, grid_size, grid_size);
            let tile_view = tile.as_view().unwrap();
            image::imageops::replace(&mut full_image, &tile_view, x * grid_size, y * grid_size);
        }
        full_image
    }
}
