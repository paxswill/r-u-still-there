// SPDX-License-Identifier: GPL-3.0-or-later
use image::{ImageBuffer, Luma, Primitive};

use std::collections::VecDeque;
use std::convert;
use std::ops;
use std::sync::{Arc, RwLock};
use std::vec::Vec;

// For now, everything in here deals with Luma pixels only. Eventually it might be nice to
// generalize them to also cover multi-channel pixels.

pub trait MovingAverage<T: 'static + Primitive, const N: usize> {
    fn update<C>(&mut self, image: &ImageBuffer<Luma<T>, C>) -> ImageBuffer<Luma<T>, Vec<T>>
    where
        C: ops::Deref<Target = [T]>,
    {
        self.push(image);
        self.to_buffer()
    }

    fn push<C>(&mut self, image: &ImageBuffer<Luma<T>, C>)
    where
        C: ops::Deref<Target = [T]>;

    fn to_buffer(&self) -> ImageBuffer<Luma<T>, Vec<T>>;
}

#[derive(Clone, Debug)]
pub struct BoxcarFilter<T, const N: usize> {
    width: u32,
    height: u32,
    frames: Arc<RwLock<VecDeque<Vec<T>>>>,
    // Possibly a premature optimization
    sums: Arc<RwLock<Vec<T>>>,
}

impl<T, const N: usize> BoxcarFilter<T, N>
where
    T: 'static + Primitive + Default,
    T: ops::AddAssign<T> + ops::SubAssign<T>,
    T: ops::DivAssign + convert::From<u16>,
{
    pub fn new(width: u32, height: u32) -> Self {
        let num_pixels = (width * height) as usize;
        Self {
            width,
            height,
            frames: Arc::new(RwLock::new(VecDeque::with_capacity(N))),
            sums: Arc::new(RwLock::new(vec![T::default(); num_pixels])),
        }
    }
}

impl<T, const N: usize> MovingAverage<T, N> for BoxcarFilter<T, N>
where
    T: 'static + Primitive + Default,
    T: ops::AddAssign<T> + ops::SubAssign<T>,
    // TODO: use a better constraint. What I'm actually trying to express is that `T: ops::Div<D>`
    // and `D: convert::From<u32>`, in other words that I can divide T by something convertable
    // from D.
    //T: ops::Div<D>,
    //D: convert::From<u32>
    T: ops::DivAssign + convert::From<u16>,
{
    fn push<C>(&mut self, image: &ImageBuffer<Luma<T>, C>)
    where
        C: ops::Deref<Target = [T]>,
    {
        // Hold write locks for this entire method
        let mut frames = self.frames.write().unwrap();
        let mut sums = self.sums.write().unwrap();
        // Always check to see if we need to pop first to keep the queue from getting too big
        if frames.len() >= N {
            if let Some(old_frame) = frames.pop_front() {
                for (old_pixel, sum_pixel) in old_frame.iter().zip(sums.iter_mut()) {
                    *sum_pixel -= *old_pixel;
                }
            }
        }
        let new_frame = image.to_vec();
        for (new_pixel, sum_pixel) in new_frame.iter().zip(sums.iter_mut()) {
            *sum_pixel += *new_pixel;
        }
        frames.push_back(new_frame);
    }

    fn to_buffer(&self) -> ImageBuffer<Luma<T>, Vec<T>> {
        // Always frames, then sums. Drop the locks as soon as the relevant data is cloned.
        let (mut averaged_frame, num_frames) = {
            let frames = self.frames.read().unwrap();
            let sums = self.sums.read().unwrap();
            let averaged_frame = sums.clone();
            (averaged_frame, frames.len())
        };
        let divisor = From::from(num_frames as u16);
        averaged_frame
            .iter_mut()
            .for_each(|pixel| *pixel /= divisor);
        ImageBuffer::from_vec(self.width, self.height, averaged_frame).unwrap()
    }
}

#[derive(Debug)]
pub struct WeightedAverage<const N: usize> {
    width: u32,
    height: u32,
    weights: [f32; N],
    frames: Arc<RwLock<VecDeque<Vec<f32>>>>,
}

impl<const N: usize> WeightedAverage<N> {
    pub fn with_weights(width: u32, height: u32, weights: [f32; N]) -> Self {
        Self {
            width,
            height,
            weights,
            frames: Arc::new(RwLock::new(VecDeque::with_capacity(N))),
        }
    }

    pub fn from_fn(width: u32, height: u32, weight_func: &dyn Fn() -> [f32; N]) -> Self {
        let raw_weights = weight_func();
        let sum: f32 = raw_weights.iter().sum();
        let mut scaled_weights = raw_weights;
        scaled_weights.iter_mut().for_each(|w| *w /= sum);
        println!("Scaled weights: {:?}", scaled_weights);
        Self::with_weights(width, height, scaled_weights)
    }
}

impl<const N: usize> MovingAverage<f32, N> for WeightedAverage<N> {
    fn push<C>(&mut self, image: &ImageBuffer<Luma<f32>, C>)
    where
        C: ops::Deref<Target = [f32]>,
    {
        // Hold a write lock for this entire method
        let mut frames = self.frames.write().unwrap();
        // Always check to see if we need to pop first to keep the queue from getting too big
        if frames.len() >= N {
            frames.pop_front();
        }
        frames.push_back(image.to_vec());
    }

    fn to_buffer(&self) -> ImageBuffer<Luma<f32>, Vec<f32>> {
        let num_pixels = (self.width * self.height) as usize;
        let mut averaged_frame = vec![f32::default(); num_pixels];
        self.frames
            .read()
            .unwrap()
            .iter()
            .zip(self.weights.iter().copied())
            .map(|(frame, weight)| frame.iter().map::<f32, _>(move |pixel| pixel * weight))
            .fold(&mut averaged_frame, |sum_frame, component_frame| {
                for (sum_pixel, component_pixel) in sum_frame.iter_mut().zip(component_frame) {
                    *sum_pixel += component_pixel;
                }
                sum_frame
            });
        ImageBuffer::from_vec(self.width, self.height, averaged_frame).unwrap()
    }
}

pub fn polynomial_weights<const WINDOW: usize>(power: f32) -> Box<dyn Fn() -> [f32; WINDOW]> {
    Box::new(move || {
        let mut weights = [f32::default(); WINDOW];
        weights.iter_mut().enumerate().for_each(|(x, dest)| {
            let x = x as f32;
            let window_length = WINDOW as f32;
            let mut weight = -((x / window_length).powf(power)) + 1.0;
            if weight.is_nan() {
                weight = 1f32;
            }
            *dest = weight;
        });
        weights
    })
}
