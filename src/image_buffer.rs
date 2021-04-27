// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::Bytes;

#[derive(Copy, Clone, Debug)]
pub enum ColorOrder {
    RGB,
    RGBA,
}

#[cfg(feature = "image")]
impl From<ColorOrder> for image::ColorType {
    fn from(value: ColorOrder) -> image::ColorType {
        match value {
            ColorOrder::RGB => image::ColorType::Rgb8,
            ColorOrder::RGBA => image::ColorType::Rgba8,
        }
    }
}

#[cfg(feature = "mozjpeg")]
impl From<ColorOrder> for mozjpeg::ColorSpace {
    fn from(value: ColorOrder) -> mozjpeg::ColorSpace {
        match value {
            ColorOrder::RGB => mozjpeg::ColorSpace::JCS_RGB,
            ColorOrder::RGBA => mozjpeg::ColorSpace::JCS_EXT_RGBA,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImageBuffer {
    data: Bytes,
    height: usize,
    width: usize,
    order: ColorOrder,
}

impl ImageBuffer {
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn order(&self) -> ColorOrder {
        self.order
    }
}

#[cfg(feature = "render_svg")]
impl From<tiny_skia::Pixmap> for ImageBuffer {
    fn from(pixmap: tiny_skia::Pixmap) -> Self {
        Self {
            data: Bytes::copy_from_slice(pixmap.data()),
            height: pixmap.height() as usize,
            width: pixmap.width() as usize,
            // tiny_skia::Pixmap always stores data as RGBA
            order: ColorOrder::RGBA,
        }
    }
}
