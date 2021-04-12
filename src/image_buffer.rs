use tiny_skia::Pixmap;

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

pub trait ImageBuffer {
    fn height(&self) -> usize;
    fn width(&self) -> usize;
    fn data(&self) -> &[u8];
    fn order(&self) -> ColorOrder;
}

impl ImageBuffer for Pixmap {
    fn height(&self) -> usize {
        self.height() as usize
    }

    fn width(&self) -> usize {
        self.width() as usize
    }

    // Pixmap already has a data() method that satisfies this trait.
    fn data(&self) -> &[u8] {
        self.data()
    }

    fn order(&self) -> ColorOrder {
        // Pixmap always stores its data as RGBA.
        ColorOrder::RGBA
    }
}
