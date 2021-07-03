// SPDX-License-Identifier: GPL-3.0-or-later
use bytes::{BufMut, Bytes, BytesMut};
use image::codecs::jpeg::JpegEncoder as ImageJpegEncoder;
use tracing::trace;

#[cfg(feature = "mozjpeg")]
use mozjpeg::{ColorSpace, Compress};

use crate::image_buffer::BytesImage;

#[cfg(feature = "mozjpeg")]
fn encode_jpeg_mozjpeg(image: &BytesImage) -> Bytes {
    trace!("using mozjpeg to encode JPEG image");
    // To make it simpler to use renderers within closures, we're creating a fresh encoder each
    // time this method is called. A little less efficient, but much easier to use.
    // BytesImage is defined to be RGBA.
    let mut jpeg_encoder = Compress::new(ColorSpace::JCS_EXT_RGBA);
    jpeg_encoder.set_color_space(ColorSpace::JCS_RGB);
    // Gotta go fast.
    // Long version: in debug builds the most time is spent in SVG rendering. In
    // optimized release builds though, JPEG encoding using the 'image' crate was taking
    // up the most time. Using mozjpeg/libjpeg-turbo will hopefully drop the CPU usage a
    // bit (and make it possible to do 10 FPS on BeagleBones/RasPi Zeros).
    jpeg_encoder.set_fastest_defaults();
    jpeg_encoder.set_quality(75.0);
    jpeg_encoder.set_mem_dest();
    jpeg_encoder.set_size(image.width() as usize, image.height() as usize);
    jpeg_encoder.start_compress();
    // Hope write_scanlines can process everything in one go.
    if !jpeg_encoder.write_scanlines(image) {
        panic!("There was an error of some kind when encoding an image to JPEG");
    }
    jpeg_encoder.finish_compress();
    Bytes::from(jpeg_encoder.data_to_vec().unwrap())
}

fn encode_jpeg_image(image: &BytesImage) -> Bytes {
    trace!("using image crate to encode JPEG image");
    let mut jpeg_buf = BytesMut::new().writer();
    let mut encoder = ImageJpegEncoder::new(&mut jpeg_buf);
    encoder.encode_image(image).unwrap();
    jpeg_buf.into_inner().freeze()
}

#[cfg(not(feature = "mozjpeg"))]
pub(crate) fn encode_jpeg(image: &BytesImage) -> Bytes {
    encode_jpeg_image(image)
}

#[cfg(feature = "mozjpeg")]
pub(crate) fn encode_jpeg(image: &BytesImage) -> Bytes {
    if cfg!(feature = "mozjpeg") {
        encode_jpeg_mozjpeg(image)
    } else {
        encode_jpeg_image(image)
    }
}
