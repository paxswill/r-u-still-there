// SPDX-License-Identifier: GPL-3.0-or-later
use futures::{Sink, Stream};
use image::{ImageBuffer, Luma};
use imageproc::region_labelling::{connected_components, Connectivity};
use itertools::Itertools;
use rayon::prelude::*;
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tracing::{debug, instrument, trace};

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::iter;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::Instant;

use crate::camera::Measurement;
use crate::image_buffer::ThermalImage;

use super::gmm::{BackgroundModel, GaussianMixtureModel};
use super::point::{Point, PointTemperature};
use super::settings::TrackerSettings;

type GmmBackground = BackgroundModel<Vec<GaussianMixtureModel>>;
type DistanceMap = HashMap<usize, HashMap<usize, f32>>;

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
        self.objects
            .read()
            .unwrap()
            .iter()
            .filter(|o| o.is_person)
            .count()
    }

    #[instrument(level = "trace", skip(self, image))]
    pub(crate) fn update(&mut self, image: &ThermalImage) {
        let mut background_option = self.background.write().unwrap();
        let background = background_option.get_or_insert_with(|| {
            let mut model = GmmBackground::new(image.len());
            model.set_parameters(self.settings.background_model_parameters);
            model
        });
        let foreground: Vec<u8> = background
            .background_probability::<Vec<f32>>(&image)
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
        let components = connected_components(&foreground, Connectivity::Eight, Luma([0u8]));
        // We only care about the foreground pixels, so skip the background (label == 0).
        let filtered_pixels = components
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel[0] != 0);
        let mut object_points: HashMap<u32, Vec<PointTemperature>> = HashMap::new();
        for (x, y, pixel) in filtered_pixels {
            let temperature = image[(x, y)][0];
            object_points
                .entry(pixel[0])
                .or_default()
                .push((Point::new(x, y), temperature));
        }
        let now = Instant::now();
        let mut new_objects: Vec<Object> = object_points
            .into_values()
            .filter_map(|points| {
                // Filter out any blobs smaller than the minimum size
                if points.len() >= self.settings.minimum_size.unwrap_or_default() {
                    Some(Object::new(points, now))
                } else {
                    debug!(point_count = %points.len(), "Skipping object because of size");
                    None
                }
            })
            .collect();
        let mut old_objects = self.objects.write().unwrap();
        let matched_objects = self.correlate_objects(&old_objects, &new_objects);
        for (old_index, new_index) in matched_objects.into_iter().enumerate() {
            let object_pair = (old_objects.get(old_index), new_objects.get_mut(new_index));
            match object_pair {
                (Some(old_object), Some(new_object)) => {
                    let old_center = old_object.center();
                    let new_center = new_object.center();
                    debug!(
                        old_object_center = ?old_center,
                        new_object_center = ?new_center,
                        "Correlated objects"
                    );
                    let center_difference = old_center.squared_distance(new_center);
                    const CENTER_CLOSE: f32 = 1.0;
                    let overlap_coefficient = old_object.overlap_coefficient(new_object);
                    const OVERLAP_THRESHOLD: f32 = 0.9;
                    // If the object hasn't moved, keep the old update time and person marking
                    if center_difference < CENTER_CLOSE && overlap_coefficient >= OVERLAP_THRESHOLD
                    {
                        new_object.last_movement = old_object.last_movement;
                        new_object.is_person = old_object.is_person;
                        debug!(object_center = ?new_center, "Ignoring movement for object")
                    } else {
                        // Conversely, if an object has moved, make sure it's marked as a person
                        new_object.is_person = true;
                        debug!(object_center = ?new_center, "Marking object as person");
                    }
                }
                (Some(old_object), None) => {
                    debug!(object_center = ?old_object.center(), "Object no longer present");
                }
                (None, Some(new_object)) => {
                    debug!(object_center = ?new_object.center(), "New object visible, waiting for movement.");
                }
                (None, None) => (),
            }
        }
        // Mark any new people, and unmark any objects that have been stationary too long.
        background.thaw_all();
        let image_width = image.width();
        for object in new_objects.iter_mut() {
            if object.last_movement.elapsed() > self.settings.stationary_timeout {
                object.is_person = false;
                let pixel_numbers = object
                    .points()
                    .map(|point| point.pixel_number(image_width) as usize)
                    .collect::<Vec<_>>();
                background.freeze_pixels(&pixel_numbers);
            }
        }
        // Update the background model, save the new objects for the next frame and broadcast the
        // new count of persons in view.
        *old_objects = new_objects;
        background.update(&image);
        // Need to release locks before count() will work
        drop(background);
        drop(background_option);
        drop(old_objects);
        let new_count = self.count();
        trace!(count = %new_count, "Current occupancy count");
        self.count_sender
            .send(new_count)
            .expect("There's a receiver also stored on the Tracker, so all sends should succeed.");
    }

    fn calculate_distances(old_objects: &[Object], new_objects: &[Object]) -> DistanceMap {
        old_objects
            .iter()
            .enumerate()
            .map(|(old_index, old)| {
                let distances_to_new = new_objects
                    .iter()
                    .enumerate()
                    .map(|(new_index, new)| {
                        let distance = old.squared_distance(new);
                        (new_index, distance)
                    })
                    .collect();
                (old_index, distances_to_new)
            })
            .collect()
    }

    #[instrument(level = "trace", skip(self, old_objects, new_objects))]
    fn correlate_objects(&self, old_objects: &[Object], new_objects: &[Object]) -> Vec<usize> {
        let distances = Self::calculate_distances(old_objects, new_objects);
        // Just brute force it. It's ugly, but there shouldn't be that many objects present at a
        // time (and the low resolution of the cameras also puts an upper limit on the complexity).
        let mut best: Option<(f32, Vec<usize>)> = None;
        let mut previous_combo: Option<Vec<usize>> = None;
        let total_combo_length = old_objects.len() + new_objects.len();
        'combo_loop: for combo_indices in (0..total_combo_length).permutations(total_combo_length) {
            // Skip repeated combos (from None being skipped)
            if previous_combo.as_ref() == Some(&combo_indices) {
                continue;
            } else {
                // Keep the previous combo indices around for the next loop iteration
                previous_combo = Some(combo_indices.clone());
            }
            let mut distance_sum = 0.0;
            for (old_index, new_index) in combo_indices.iter().enumerate() {
                match (old_objects.get(old_index), new_objects.get(*new_index)) {
                    (Some(_), Some(_)) => {
                        let distance = distances[&old_index][new_index];
                        if distance > self.settings.maximum_movement {
                            continue 'combo_loop;
                        }
                        // Guard against a divide by zero
                        let distance = if distance != 0.0 {
                            // Using reciprocal distance so that not moving is weighted heavier
                            // than moving.
                            distance.recip()
                        } else {
                            0.0
                        };
                        distance_sum += distance;
                    }
                    _ => distance_sum += 1.0,
                }
            }
            let average_distance = distance_sum / total_combo_length as f32;
            let current_best = best.as_ref().map(|(d, _)| *d).unwrap_or(f32::INFINITY);
            if average_distance < current_best {
                trace!(new_score = ?average_distance, "Found new lowest score");
                best = Some((average_distance, combo_indices));
            }
        }
        best.map(|(_, mapping)| mapping.into_iter().collect())
            .unwrap_or_default()
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

#[derive(Clone, Debug)]
struct Object {
    point_temperatures: Vec<PointTemperature>,
    last_movement: Instant,
    is_person: bool,
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
            is_person: false,
        }
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

    pub(crate) fn squared_distance(&self, other: &Self) -> f32 {
        // Combine Bhattacharyya (for the thermal distribution) and euclidean (for the center
        // coordinates) distances.
        let mean = (self.temperature_mean(), other.temperature_mean());
        let variance = (self.temperature_variance(), other.temperature_variance());
        // I'm sorry.
        let bhattacharyya_distance = 0.25
            * (0.25 * (variance.0 / variance.1 + variance.1 / variance.0 + 2.0)).ln()
            + 0.25 * ((mean.0 - mean.1).powi(2) / (variance.0 + variance.1));
        let center_distance = self.center().squared_distance(other.center());
        trace!(
            ?bhattacharyya_distance,
            ?center_distance,
            a = ?self,
            b = ?other,
            "Distance between objects"
        );
        bhattacharyya_distance + center_distance
    }

    fn overlap_coefficient(&self, other: &Self) -> f32 {
        let this = self.points().copied().collect::<HashSet<Point<_>>>();
        let that = other.points().copied().collect::<HashSet<Point<_>>>();
        let intersection = this.intersection(&that).count();
        let denom = this.len().min(that.len());
        intersection as f32 / denom as f32
    }
}

#[cfg(test)]
mod test {
    use float_cmp::assert_approx_eq;

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
