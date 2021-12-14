// SPDX-License-Identifier: GPL-3.0-or-later
use futures::{Sink, Stream};
use image::{ImageBuffer, Luma};
use imageproc::region_labelling::{connected_components, Connectivity};
use rayon::prelude::*;
use rstar::{Envelope, PointDistance, RTree, RTreeObject};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tracing::{debug, debug_span, instrument, trace};

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
use super::moments::hu_moments;
use super::point::{Point, PointTemperature};
use super::settings::TrackerSettings;

type GmmBackground = BackgroundModel<Vec<GaussianMixtureModel>>;

#[derive(Clone, Debug)]
pub(crate) struct Tracker {
    settings: TrackerSettings,
    background: Arc<RwLock<Option<GmmBackground>>>,
    objects: Arc<RwLock<RTree<Object>>>,
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
            objects: Arc::new(RwLock::new(RTree::default())),
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
            .background_probability::<Vec<f32>>(image)
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
        let new_objects: Vec<Object> = object_points
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
        let mut new_objects: RTree<Object> = RTree::bulk_load(new_objects);
        let mut old_objects = self.objects.write().unwrap();
        self.update_tracked_objects(&mut old_objects, &mut new_objects);
        // Mark any new people, and unmark any objects that have been stationary too long.
        background.thaw_all();
        let image_width = image.width();
        for object in new_objects.iter_mut() {
            if object.is_person {
                if object.last_movement.elapsed() > self.settings.stationary_timeout {
                    object.is_person = false;
                } else {
                    let pixel_numbers = object
                        .points()
                        .map(|point| point.pixel_number(image_width) as usize)
                        .collect::<Vec<_>>();
                    background.freeze_pixels(&pixel_numbers);
                }
            }
        }
        // Update the background model, save the new objects for the next frame and broadcast the
        // new count of persons in view.
        *old_objects = new_objects;
        background.update(image);
        // Need to release locks before count() will work
        drop(background_option);
        drop(old_objects);
        let new_count = self.count();
        trace!(count = %new_count, "Current occupancy count");
        self.count_sender
            .send(new_count)
            .expect("There's a receiver also stored on the Tracker, so all sends should succeed.");
    }

    #[instrument(
        name = "object_tracking",
        level = "debug",
        skip(self, old_objects, new_objects)
    )]
    fn update_tracked_objects(
        &self,
        old_objects: &mut RTree<Object>,
        new_objects: &mut RTree<Object>,
    ) {
        let max_distance = self.settings.maximum_movement;
        for new_object in new_objects.iter_mut() {
            let neighbor = old_objects.pop_nearest_neighbor(&new_object.hu_moments);
            if let Some(old_object) = neighbor {
                let neighbor_distance =
                    new_object.distance_2_if_less_or_equal(&old_object.hu_moments, max_distance);
                if let Some(distance_2) = neighbor_distance {
                    let object_pair_span = debug_span!("Correlated objects");
                    let _pair_span = object_pair_span.enter();
                    debug!(
                        old_object = %old_object.summary(),
                        new_object = %new_object.summary(),
                        %distance_2,
                    );
                    let old_center = old_object.center();
                    let new_center = new_object.center();
                    let center_difference = old_center.squared_distance(new_center);
                    let overlap_coefficient = old_object.overlap_coefficient(new_object);
                    // If the object hasn't moved, keep the old update time and person marking
                    trace!(%center_difference, %overlap_coefficient);
                    if center_difference < self.settings.center_closeness
                        && overlap_coefficient >= self.settings.overlap_threshold
                    {
                        new_object.last_movement = old_object.last_movement;
                        new_object.is_person = old_object.is_person;
                        debug!("Ignoring movement for object");
                    } else {
                        // Conversely, if an object has moved, make sure it's marked as a person
                        new_object.is_person = true;
                        debug!("Marking object as person");
                    }
                } else {
                    // Put the old object back in if it's too far away.
                    old_objects.insert(old_object);
                }
            }
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

#[derive(Clone, Debug)]
struct Object {
    point_temperatures: Vec<PointTemperature>,
    hu_moments: [f32; 7],
    last_movement: Instant,
    is_person: bool,
}

impl Object {
    fn new<I>(point_temperatures: I, when: Instant) -> Self
    where
        I: IntoIterator<Item = PointTemperature>,
    {
        let point_temperatures: Vec<PointTemperature> = point_temperatures.into_iter().collect();
        let hu_moments = hu_moments(&point_temperatures);
        assert!(
            !point_temperatures.is_empty(),
            "An object must have at least one point"
        );
        Self {
            point_temperatures,
            hu_moments,
            last_movement: when,
            is_person: false,
        }
    }

    fn summary(&self) -> String {
        let center = self.center();
        format!(
            "Point(center: ({:5.2}, {:5.2}), human: {:3}, last_movement: {:5.1}s ago)",
            center.x,
            center.y,
            if self.is_person { "yes" } else { "no" },
            self.last_movement.elapsed().as_secs_f32(),
        )
    }

    fn len(&self) -> usize {
        self.point_temperatures.len()
    }

    fn points(&self) -> impl iter::ExactSizeIterator<Item = &Point<u32>> {
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
        // Just compute the squared euclidean distance from this object's Hu moments to the othe
        // object's Hu moments.
        self.hu_moments
            .iter()
            .zip(other.hu_moments.iter())
            .map(|(this, that)| (this - that).powi(2))
            .sum()
    }

    fn overlap_coefficient(&self, other: &Self) -> f32 {
        let this = self.points().copied().collect::<HashSet<Point<_>>>();
        let that = other.points().copied().collect::<HashSet<Point<_>>>();
        let intersection = this.intersection(&that).count();
        let denom = this.len().min(that.len());
        intersection as f32 / denom as f32
    }
}

impl RTreeObject for Object {
    type Envelope = rstar::AABB<[f32; 7]>;

    fn envelope(&self) -> Self::Envelope {
        rstar::AABB::from_point(self.hu_moments)
    }
}

impl PointDistance for Object {
    fn distance_2(
        &self,
        point: &<Self::Envelope as Envelope>::Point,
    ) -> <<Self::Envelope as Envelope>::Point as rstar::Point>::Scalar {
        self.hu_moments.distance_2(point)
    }

    fn contains_point(&self, point: &<Self::Envelope as Envelope>::Point) -> bool {
        self.hu_moments.contains_point(point)
    }

    fn distance_2_if_less_or_equal(
        &self,
        point: &<Self::Envelope as Envelope>::Point,
        max_distance_2: <<Self::Envelope as Envelope>::Point as rstar::Point>::Scalar,
    ) -> Option<<<Self::Envelope as Envelope>::Point as rstar::Point>::Scalar> {
        self.hu_moments
            .distance_2_if_less_or_equal(point, max_distance_2)
    }
}

#[cfg(test)]
mod test {
    use std::io::Cursor;
    use std::time::Instant;

    use float_cmp::assert_approx_eq;

    use crate::occupancy::TrackerSettings;
    use crate::recorded_data::RecordedData;

    use super::{Object, Point, PointTemperature, Tracker};

    const EMPTY_ROOM_DATA: &[u8] = include_bytes!("empty-room.bin");
    const WALK_IN_DATA: &[u8] = include_bytes!("walk-in.bin");
    const WARM_UP_DATA: &[u8] = include_bytes!("warm-up.bin");
    const PERSON_OVERLAP_DATA: &[u8] = include_bytes!("person-overlap.bin");

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

    struct OccupancyCount {
        count: usize,
        start_frame: Option<usize>,
        end_frame: Option<usize>,
    }

    impl OccupancyCount {
        fn new_unbounded(count: usize) -> Self {
            Self {
                count,
                start_frame: None,
                end_frame: None,
            }
        }

        fn new_until(count: usize, until_frame: usize) -> Self {
            Self {
                count,
                start_frame: None,
                end_frame: Some(until_frame),
            }
        }

        fn new_from(count: usize, from_frame: usize) -> Self {
            Self {
                count,
                start_frame: Some(from_frame),
                end_frame: None,
            }
        }

        fn new(count: usize, start: usize, end: usize) -> Self {
            Self {
                count,
                start_frame: Some(start),
                end_frame: Some(end),
            }
        }

        fn contains(&self, frame_number: usize) -> bool {
            match (self.start_frame, self.end_frame) {
                (None, None) => true,
                (None, Some(end_frame)) => frame_number < end_frame,
                (Some(start_frame), None) => start_frame <= frame_number,
                (Some(start_frame), Some(end_frame)) => {
                    (start_frame..end_frame).contains(&frame_number)
                }
            }
        }

        fn matches_count(&self, frame_number: usize, count: usize) -> Option<bool> {
            if self.contains(frame_number) {
                Some(self.count == count)
            } else {
                None
            }
        }
    }

    // Test the tests a bit here...

    #[test]
    fn test_count_unbounded() {
        const EXPECTED_COUNT: usize = 2;
        let count = OccupancyCount::new_unbounded(EXPECTED_COUNT);
        assert!(
            count.contains(usize::MIN),
            "An unbounded count includes the minimum value"
        );
        assert!(
            count.contains(usize::MAX),
            "An unbounded count includes the maximum value"
        );
        assert_eq!(count.matches_count(usize::MIN, EXPECTED_COUNT), Some(true));
        assert_eq!(count.matches_count(usize::MAX, EXPECTED_COUNT), Some(true));
        assert_eq!(count.matches_count(usize::MIN, 0), Some(false));
        assert_eq!(count.matches_count(usize::MAX, 0), Some(false));
    }

    #[test]
    fn test_count_upper_bound() {
        const EXPECTED_COUNT: usize = 2;
        const UPPER_BOUND: usize = 10;
        let count = OccupancyCount::new_until(EXPECTED_COUNT, UPPER_BOUND);
        assert!(
            count.contains(usize::MIN),
            "An upper bounded count includes the minimum value"
        );
        assert!(
            count.contains(UPPER_BOUND - 1),
            "An upper bounded count includes up to its bound"
        );
        assert!(
            !count.contains(UPPER_BOUND),
            "An upper bounded count does not include the bound itself"
        );
        assert_eq!(count.matches_count(usize::MIN, EXPECTED_COUNT), Some(true));
        assert_eq!(
            count.matches_count(UPPER_BOUND - 1, EXPECTED_COUNT),
            Some(true)
        );
        assert_eq!(count.matches_count(UPPER_BOUND, EXPECTED_COUNT), None);
        assert_eq!(count.matches_count(usize::MIN, 0), Some(false));
        assert_eq!(count.matches_count(UPPER_BOUND - 1, 0), Some(false));
    }

    #[test]
    fn test_count_lower_bound() {
        const EXPECTED_COUNT: usize = 2;
        const LOWER_BOUND: usize = 10;
        let count = OccupancyCount::new_from(EXPECTED_COUNT, LOWER_BOUND);
        assert!(
            count.contains(usize::MAX),
            "A lower bounded count includes the maximum value"
        );
        assert!(
            count.contains(LOWER_BOUND),
            "A lower count includes the lower bound"
        );
        assert!(
            !count.contains(LOWER_BOUND - 1),
            "A lower bounded count does not include a value below the bound"
        );
        assert_eq!(count.matches_count(usize::MAX, EXPECTED_COUNT), Some(true));
        assert_eq!(count.matches_count(LOWER_BOUND, EXPECTED_COUNT), Some(true));
        assert_eq!(count.matches_count(LOWER_BOUND - 1, EXPECTED_COUNT), None);
        assert_eq!(count.matches_count(usize::MAX, 0), Some(false));
        assert_eq!(count.matches_count(LOWER_BOUND, 0), Some(false));
    }

    #[test]
    fn test_count_bounded() {
        const EXPECTED_COUNT: usize = 2;
        const LOWER_BOUND: usize = 10;
        const UPPER_BOUND: usize = 20;
        let count = OccupancyCount::new(EXPECTED_COUNT, LOWER_BOUND, UPPER_BOUND);
        assert!(
            count.contains(LOWER_BOUND),
            "A bounded count includes the lower bounds"
        );
        assert!(
            !count.contains(UPPER_BOUND),
            "A bounded count does not include the upper bound"
        );
        assert!(
            !count.contains(usize::MIN),
            "A bounded count does not include anything below the lower bound"
        );
        assert!(
            count.contains(UPPER_BOUND - 1),
            "A bounded count includes up to the upper bounds"
        );
        assert_eq!(count.matches_count(usize::MIN, 0), None);
        assert_eq!(count.matches_count(usize::MAX, 0), None);
        assert_eq!(count.matches_count(LOWER_BOUND, EXPECTED_COUNT), Some(true));
        assert_eq!(count.matches_count(LOWER_BOUND, 0), Some(false));
        assert_eq!(count.matches_count(LOWER_BOUND - 1, EXPECTED_COUNT), None);
        assert_eq!(
            count.matches_count(UPPER_BOUND - 1, EXPECTED_COUNT),
            Some(true)
        );
        assert_eq!(count.matches_count(UPPER_BOUND - 1, 0), Some(false));
    }

    fn check_recorded_data(
        recorded_data: &[RecordedData],
        occupancy_counts: &[OccupancyCount],
        settings: &TrackerSettings,
    ) -> bool {
        let mut tracker = Tracker::new(settings);
        let mut failed = false;
        for (frame_number, record) in recorded_data.iter().enumerate() {
            tracker.update(&record.measurement.image);
            let tracker_count = tracker.count();
            let occupancy_range = occupancy_counts.iter().find(|o| o.contains(frame_number));
            if let Some(occupancy_range) = occupancy_range {
                if tracker_count != occupancy_range.count {
                    println!(
                        "Frame #{}: {} people detected (should be {})",
                        frame_number, tracker_count, occupancy_range.count
                    );
                    failed = true;
                }
            }
        }
        !failed
    }

    #[test]
    fn empty_room() {
        let recorded_data = RecordedData::from_bincode(Cursor::new(EMPTY_ROOM_DATA))
            .expect("Decoding test data should work");
        let settings = TrackerSettings::default();
        let expected_counts = vec![OccupancyCount::new_unbounded(0)];
        assert!(
            check_recorded_data(&recorded_data, &expected_counts, &settings),
            "People detected in empty room"
        );
    }

    #[test]
    #[ignore = "object tracking unreliable"]
    fn walk_in() {
        let recorded_data = RecordedData::from_bincode(Cursor::new(WALK_IN_DATA))
            .expect("Decoding test data should work");
        let settings = TrackerSettings::default();
        let expected_counts = vec![
            OccupancyCount::new_until(0, 6003),
            OccupancyCount::new(1, 6064, 8355),
            OccupancyCount::new_from(0, 8399),
        ];
        assert!(check_recorded_data(
            &recorded_data,
            &expected_counts,
            &settings
        ));
    }

    #[test]
    #[ignore = "object tracking unreliable"]
    fn warm_up() {
        let recorded_data = RecordedData::from_bincode(Cursor::new(WARM_UP_DATA))
            .expect("Decoding test data should work");
        let settings = TrackerSettings::default();
        let expected_counts = vec![
            OccupancyCount::new_until(0, 2039),
            OccupancyCount::new_from(1, 2040),
        ];
        assert!(check_recorded_data(
            &recorded_data,
            &expected_counts,
            &settings
        ));
    }

    #[test]
    #[ignore = "object tracking unreliable"]
    fn person_overlap() {
        let recorded_data = RecordedData::from_bincode(Cursor::new(PERSON_OVERLAP_DATA))
            .expect("Decoding test data should work");
        let settings = TrackerSettings::default();
        let expected_counts = vec![
            OccupancyCount::new_until(0, 2786),
            OccupancyCount::new(1, 2787, 2996),
            OccupancyCount::new(2, 2997, 3165),
            OccupancyCount::new(1, 3166, 8278),
            OccupancyCount::new_from(0, 8279),
        ];
        assert!(check_recorded_data(
            &recorded_data,
            &expected_counts,
            &settings
        ));
    }
}
