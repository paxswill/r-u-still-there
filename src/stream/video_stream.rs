// SPDX-License-Identifier: GPL-3.0-or-later
use crate::image_buffer::ImageBuffer;

pub trait VideoStream<E> {
    fn send_frame(&mut self, buf: &ImageBuffer) -> Result<usize, E>;
}
