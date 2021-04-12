use crate::image_buffer::ImageBuffer;

pub trait VideoStream<E> {
    fn send_frame(&mut self, buf: &dyn ImageBuffer) -> Result<(), E>;
}
