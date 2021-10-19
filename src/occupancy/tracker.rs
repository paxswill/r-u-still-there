// SPDX-License-Identifier: GPL-3.0-or-later
use futures::{Sink, Stream};
use image::{ImageBuffer, Luma};
use imageproc::point::Point;
use imageproc::region_labelling::{connected_components, Connectivity};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tracing::debug;

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};

use crate::camera::Measurement;
use crate::image_buffer::ThermalImage;

use super::gmm::{BackgroundModel, GaussianMixtureModel, GmmParameters};
use super::settings::TrackerSettings;

type GmmBackground = BackgroundModel<GaussianMixtureModel, Vec<GaussianMixtureModel>>;

#[derive(Clone, Debug)]
pub(crate) struct Tracker {
    model_settings: GmmParameters,
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
            model_settings: settings.background_model_parameters,
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
            model.set_parameters(self.model_settings);
            model
        });
        // TODO: Add detection of previously moving people. Until then there's no object tracking,
        // just background subtraction.
        // TODO: Have a configurable threshold here
        const BG_CONFIDENCE_THRESHOLD: f32 = 0.001;
        let classified: Vec<u8> = background
            .update_and_classify::<Vec<f32>>(&image)
            .into_iter()
            .map(|p| {
                if p < BG_CONFIDENCE_THRESHOLD {
                    u8::MAX
                } else {
                    0u8
                }
            })
            .collect();
        let classified: ImageBuffer<Luma<u8>, Vec<u8>> =
            ImageBuffer::from_raw(image.width(), image.height(), classified)
                .expect("A mapped Vec should be able to be used for a new ImageBuffer");
        let mut object_points: HashMap<u32, Vec<Point<u32>>> = HashMap::new();
        let components = connected_components(&classified, Connectivity::Eight, Luma([0u8]));
        // We only care about the foreground pixels, so skip the background (label == 0).
        let filtered_pixels = components
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel[0] != 0);
        for (x, y, pixel) in filtered_pixels {
            object_points
                .entry(pixel[0])
                .or_default()
                .push(Point::new(x, y));
        }
        let objects: Vec<Object> = object_points
            .values()
            .map(|points| points.iter().cloned().collect())
            .collect();
        {
            let mut locked_objects = self.objects.write().unwrap();
            let new_count = objects.len();
            *locked_objects = objects;
            self.count_sender.send(new_count);
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
    points: Vec<Point<u32>>,
}

impl Object {
    pub(crate) fn center(&self) -> Option<Point<u32>> {
        // Short circuit the easy cases
        match self.points.len() {
            0 => None,
            1 => Some(self.points[0]),
            _ => {
                let mut min_x = u32::MAX;
                let mut min_y = u32::MAX;
                let mut max_x = u32::MIN;
                let mut max_y = u32::MIN;
                for point in self.points.iter() {
                    min_y = point.y.min(min_y);
                    min_x = point.x.min(min_x);
                    max_y = point.y.max(max_y);
                    max_x = point.x.max(max_x);
                }
                Some(Point::new((max_x - min_x) / 2, (max_y - min_y) / 2))
            }
        }
    }
}

impl std::iter::FromIterator<Point<u32>> for Object {
    fn from_iter<I: IntoIterator<Item = Point<u32>>>(iter: I) -> Self {
        Self {
            points: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::{Object, Point};

    #[test]
    fn center_empty() {
        let points: [Point<u32>; 0] = [];
        let object: Object = std::array::IntoIter::new(points).collect();
        assert_eq!(
            object.center(),
            None,
            "An empty object doesn't have a center"
        );
    }

    #[test]
    fn center_single() {
        let points: [Point<u32>; 1] = [Point::new(3, 9)];
        let object: Object = std::array::IntoIter::new(points).collect();
        assert_eq!(
            object.center(),
            Some(Point::new(3, 9)),
            "A object with a single point should have the same center"
        );
    }

    #[test]
    fn center_multiple() {
        let points: [Point<u32>; 6] = [
            // A rectangle, but with extra points that're within the box to ensure it's not just
            // averaging all points. A rtectangle is used to ensure both dimensions are being
            // looked at separately.
            Point::new(0, 0),
            Point::new(0, 10),
            Point::new(1, 1),
            Point::new(3, 2),
            Point::new(4, 0),
            Point::new(4, 10),
        ];
        let object: Object = std::array::IntoIter::new(points).collect();
        assert_eq!(
            object.center(),
            Some(Point::new(2, 5)),
            "Incorrect center for a rectangle with bounding box ((0, 0), (4, 10)"
        );
    }
}
