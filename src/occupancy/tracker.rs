// SPDX-License-Identifier: GPL-3.0-or-later
use futures::{Sink, Stream};
use image::{ImageBuffer, Luma};
use imageproc::point::Point;
use imageproc::region_labelling::{connected_components, Connectivity};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};

use super::Threshold;
use crate::image_buffer::ThermalImage;

#[derive(Clone, Debug)]
pub struct Tracker {
    threshold: Threshold,
    blobs: Arc<RwLock<Vec<Blob>>>,
    old_count: Arc<AtomicUsize>,
    count_sender: Arc<watch::Sender<usize>>,
    count_receiver: watch::Receiver<usize>,
}

#[derive(Clone, Debug)]
struct Blob {
    points: Vec<Point<u32>>,
}

impl Tracker {
    pub fn new(threshold: f32) -> Self {
        let (sender, receiver) = watch::channel(0);
        Self {
            threshold: Threshold::Static(threshold),
            blobs: Arc::new(RwLock::new(Vec::default())),
            old_count: Arc::new(AtomicUsize::new(0)),
            count_sender: Arc::new(sender),
            count_receiver: receiver,
        }
    }

    pub fn count(&self) -> usize {
        self.blobs.read().unwrap().len()
    }

    pub fn update(&mut self, image: &ThermalImage) {
        let thresholded: ImageBuffer<Luma<u8>, Vec<u8>> = self.threshold.threshold_image(image);
        let mut blob_points: HashMap<u32, Vec<Point<u32>>> = HashMap::new();
        let components = connected_components(&thresholded, Connectivity::Eight, Luma([0u8]));
        // We only care about the foreground pixels, so skip the background (label == 0).
        let filtered_pixels = components
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel[0] != 0);
        for (x, y, pixel) in filtered_pixels {
            blob_points
                .entry(pixel[0])
                .or_default()
                .push(Point::new(x, y));
        }
        let blobs: Vec<Blob> = blob_points
            .values()
            .map(|points| points.iter().cloned().collect())
            .collect();
        {
            let mut locked_blobs = self.blobs.write().unwrap();
            let new_count = blobs.len();
            *locked_blobs = blobs;
            if new_count != self.old_count.load(Ordering::Acquire) {
                self.count_sender
                    .send(new_count)
                    .expect("Sending updated count failed");
                self.old_count.store(new_count, Ordering::Release);
            }
        }
    }

    pub fn count_stream(&self) -> impl Stream<Item = usize> {
        WatchStream::new(self.count_receiver.clone())
    }
}

impl Default for Tracker {
    fn default() -> Self {
        // Using 26.0 degrees celsius as the default threshold, compensating for the effect of
        // clothing and distance on normal body temperature (37 degrees) while still being a fairly
        // high ambient temperature.
        Self::new(25.0)
    }
}

impl Sink<ThermalImage> for Tracker {
    type Error = Infallible;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Always ready to receive new frames
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, image: ThermalImage) -> Result<(), Self::Error> {
        self.update(&image);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

impl Blob {
    fn center(&self) -> Point<u32> {
        todo!();
    }
}

impl std::iter::FromIterator<Point<u32>> for Blob {
    fn from_iter<I: IntoIterator<Item = Point<u32>>>(iter: I) -> Self {
        Self {
            points: iter.into_iter().collect(),
        }
    }
}
