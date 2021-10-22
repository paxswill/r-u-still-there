// SPDX-License-Identifier: GPL-3.0-or-later
use imageproc::point::Point as ImagePoint;
use num_traits::Num;

pub(super) type PointTemperature = (Point<u32>, f32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct Point<T: Num> {
    pub x: T,
    pub y: T,
}

impl<T: Num> Point<T> {
    pub(super) fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

impl Point<u32> {
    pub(super) fn pixel_number(&self, image_width: u32) -> u32 {
        self.x + self.y * image_width
    }
}

impl Point<f32> {
    pub(super) fn squared_distance(&self, other: Self) -> f32 {
        (self.x - other.x).powi(2) + (self.y - other.y).powi(2)
    }
}

impl<T: Num> From<Point<T>> for ImagePoint<T> {
    fn from(pt: Point<T>) -> Self {
        Self { x: pt.x, y: pt.y }
    }
}

impl<T: Num> From<ImagePoint<T>> for Point<T> {
    fn from(image_point: ImagePoint<T>) -> Self {
        Self {
            x: image_point.x,
            y: image_point.y,
        }
    }
}
