// SPDX-License-Identifier: GPL-3.0-or-later
use futures::{Sink, Stream};
use image::{ImageBuffer, Luma};
use imageproc::point::Point as ImagePoint;
use imageproc::region_labelling::{connected_components, Connectivity};
use num_traits::Num;
use rayon::prelude::*;
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tracing::debug;

use std::collections::HashMap;
use std::cmp;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::Instant;

use crate::camera::Measurement;
use crate::image_buffer::ThermalImage;

use super::gmm::{BackgroundModel, GaussianMixtureModel};
use super::settings::TrackerSettings;

type GmmBackground = BackgroundModel<Vec<GaussianMixtureModel>>;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Point<T: Num> {
    x: T,
    y: T,
}

impl<T: Num> Point<T> {
    fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

impl Point<u32> {
    fn squared_distance(&self, other: &Self) -> u32 {
        let diff_x = cmp::max(self.x, other.x) - cmp::min(self.x, other.x);
        let diff_y = cmp::max(self.y, other.y) - cmp::min(self.y, other.y);
        diff_x.pow(2) + diff_y.pow(2)
    }
}

impl Point<f32> {
    fn squared_distance(&self, other: &Self) -> f32 {
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

#[derive(Clone, Debug)]
pub(crate) struct Tracker {
    settings: TrackerSettings,
    background: Arc<RwLock<Option<GmmBackground>>>,
    objects: Arc<RwLock<Vec<Object>>>,
    count_sender: Arc<watch::Sender<usize>>,
    count_receiver: watch::Receiver<usize>,
}

impl Tracker {
    pub(crate) fn new(settings: &TrackerSettings) -> Self {
        debug!(params=?settings.background_model_parameters, "GMM parameters");
        let (sender, receiver) = watch::channel(0);
        Self {
            settings: *settings,
            background: Arc::new(RwLock::new(None)),
            objects: Arc::new(RwLock::new(Vec::default())),
            count_sender: Arc::new(sender),
            count_receiver: receiver,
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.objects.read().unwrap().len()
    }

    pub(crate) fn update(&mut self, image: &ThermalImage) {
        let mut background_option = self.background.write().unwrap();
        let background = background_option.get_or_insert_with(|| {
            let mut model = GmmBackground::new(image.len());
            model.set_parameters(self.settings.background_model_parameters);
            model
        });
        // TODO: Add detection of previously moving people. Until then there's no object tracking,
        // just background subtraction.
        let foreground: Vec<u8> = background
            .update_and_classify::<Vec<f32>>(&image)
            .into_iter()
            .map(|p| {
                if p < self.settings.background_confidence_threshold {
                    u8::MAX
                } else {
                    0u8
                }
            })
            .collect();
        let foreground: ImageBuffer<Luma<u8>, Vec<u8>> =
            ImageBuffer::from_raw(image.width(), image.height(), foreground)
                .expect("A mapped Vec should be able to be used for a new ImageBuffer");
        let mut object_points: HashMap<u32, Vec<PointTemperature>> = HashMap::new();
        let components = connected_components(&foreground, Connectivity::Eight, Luma([0u8]));
        // We only care about the foreground pixels, so skip the background (label == 0).
        let filtered_pixels = components
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel[0] != 0);
        for (x, y, pixel) in filtered_pixels {
            let temperature = image[(x, y)][0];
            object_points
                .entry(pixel[0])
                .or_default()
                .push((Point::new(x, y), temperature));
        }
        let now = Instant::now();
        let new_objects: Vec<Object> = object_points
            .into_values()
            .filter_map(|points| {
                // Filter out any blobs smaller than the minimum size
                if points.len() < self.settings.minimum_size.unwrap_or_default() {
                    Some(Object::new(points, now))
                } else {
                    None
                }
            })
            .collect();
        {
            let mut locked_objects = self.objects.write().unwrap();
            let new_count = new_objects.len();
            *locked_objects = new_objects;
            self.count_sender.send(new_count).expect(
                "There's a receiver also stored on the Tracker, so all sends should succeed.",
            );
        }
    }

    pub(crate) fn count_stream(&self) -> impl Stream<Item = usize> {
        WatchStream::new(self.count_receiver.clone())
    }
}

impl Sink<Measurement> for Tracker {
    type Error = Infallible;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Always ready to receive new frames
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, measurement: Measurement) -> Result<(), Self::Error> {
        self.update(&measurement.image);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

type PointTemperature = (Point<u32>, f32);

#[derive(Clone, Debug)]
struct Object {
    point_temperatures: Vec<PointTemperature>,
    last_movement: Instant,
}

impl Object {
    fn new<I>(point_temperatures: I, when: Instant) -> Self
    where
        I: IntoIterator<Item = PointTemperature>,
    {
        let point_temperatures: Vec<PointTemperature> = point_temperatures.into_iter().collect();
        assert!(
            !point_temperatures.is_empty(),
            "An object must have at least one point"
        );
        Self {
            point_temperatures,
            last_movement: when,
        }
    }

    fn set_last_movement(&mut self, when: Instant) {
        self.last_movement = when
    }

    fn len(&self) -> usize {
        self.point_temperatures.len()
    }

    fn points<'a>(&'a self) -> impl iter::ExactSizeIterator<Item = &'a Point<u32>> {
        self.point_temperatures.iter().map(|(p, _)| p)
    }

    pub(crate) fn center(&self) -> Point<f32> {
        let mut points = self
            .points()
            .map(|point| Point::new(point.x as f32, point.y as f32));
        // Short circuit the easy cases
        match self.len() {
            0 => unreachable!("There must always be at least one point in an object"),
            1 => points.next().unwrap(),
            _ => {
                let mut min_x = f32::MAX;
                let mut min_y = f32::MAX;
                let mut max_x = f32::MIN;
                let mut max_y = f32::MIN;
                for point in points {
                    min_y = point.y.min(min_y);
                    min_x = point.x.min(min_x);
                    max_y = point.y.max(max_y);
                    max_x = point.x.max(max_x);
                }
                Point::new((max_x - min_x) / 2.0, (max_y - min_y) / 2.0)
            }
        }
    }

    fn sum_temperatures(&self) -> f32 {
        self.point_temperatures
            .par_iter()
            .map(|(_, temperature)| temperature)
            .sum()
    }

    pub(crate) fn temperature_mean(&self) -> f32 {
        self.sum_temperatures() / self.len() as f32
    }

    pub(crate) fn temperature_variance(&self) -> f32 {
        let mean = self.temperature_mean();
        let squared_deviations_sum: f32 = self
            .point_temperatures
            .par_iter()
            .map(|(_, temperature)| (temperature - mean).powi(2))
            .sum();
        squared_deviations_sum / self.len() as f32
    }
    }
}

#[cfg(test)]
mod test {
    use float_cmp::{assert_approx_eq, F32Margin};

    use std::time::Instant;

    use super::{Object, Point, PointTemperature};

    #[test]
    fn single_object_stats() {
        let points: [PointTemperature; 1] = [(Point::new(3, 9), 37.0)];
        let object = Object::new(points, Instant::now());
        assert_eq!(
            object.center(),
            Point::new(3.0, 9.0),
            "A object with a single point should have the same center"
        );
        assert_eq!(
            object.temperature_mean(),
            37.0,
            "An object with only one temperature has that temperature as the mean"
        );
        assert_eq!(
            object.temperature_variance(),
            0.0,
            "An object with only one point doesn't have a temperature variance"
        );
    }

    #[test]
    fn multi_point_object_stats() {
        let points: [PointTemperature; 6] = [
            // A rectangle, but with extra points that're within the box to ensure it's not just
            // averaging all points. A rectangle is used to ensure both dimensions are being
            // looked at separately.
            (Point::new(0, 0), 37.26),
            (Point::new(0, 10), 36.71),
            (Point::new(1, 1), 36.98),
            (Point::new(3, 2), 37.34),
            (Point::new(4, 0), 36.88),
            (Point::new(4, 10), 36.71),
        ];
        let object = Object::new(points, Instant::now());
        // Manually calculated (well, in Excel)
        const MEAN: f32 = 36.98;
        const VARIANCE: f32 = 0.0606;
        assert_eq!(
            object.center(),
            Point::new(2.0, 5.0),
            "Incorrect center for a rectangle with bounding box ((0, 0), (4, 10)"
        );
        let mean = object.temperature_mean();
        assert_approx_eq!(f32, mean, MEAN, epsilon = 0.01);
        let variance = object.temperature_variance();
        assert_approx_eq!(f32, variance, VARIANCE, epsilon = 0.0001);
    }
}
