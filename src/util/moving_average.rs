// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::VecDeque;
use std::convert;
use std::ops::{self, Deref, DerefMut};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::vec::Vec;

use bytes::{Bytes, BytesMut};
use image::{ImageBuffer, Pixel, Primitive};

/// A trait for types that can be averaged together.
pub trait Average<Div, Result = Self> {
    fn add(&self, rhs: &Self) -> Result;
    fn sub(&self, rhs: &Self) -> Result;
    fn div(&self, rhs: &Div) -> Result;
}

/// A trait for types that can be averaged together in-place.
pub trait AverageMut<Div, Result = Self>: Average<Div, Result> {
    fn add_assign(&mut self, rhs: &Self);
    fn sub_assign(&mut self, rhs: &Self);
}

macro_rules! average_primitive {
    ($primitive:tt) => {
        impl<Div> Average<Div> for $primitive
        where
            Div: convert::Into<$primitive> + Copy,
        {
            fn add(&self, rhs: &Self) -> Self {
                self + rhs
            }
            fn sub(&self, rhs: &Self) -> Self {
                self - rhs
            }
            fn div(&self, rhs: &Div) -> Self {
                let divisor: $primitive = Into::<$primitive>::into(*rhs);
                self / divisor
            }
        }
        impl<Div> AverageMut<Div> for $primitive
        where
            Div: convert::Into<$primitive> + Copy,
        {
            fn add_assign(&mut self, rhs: &Self) {
                *self += *rhs;
            }

            fn sub_assign(&mut self, rhs: &Self) {
                *self -= rhs;
            }
        }
    };
}

average_primitive!(f32);
average_primitive!(f64);
average_primitive!(u8);
average_primitive!(u16);
average_primitive!(u32);

impl Average<u16> for Duration {
    fn add(&self, rhs: &Self) -> Self {
        *self + *rhs
    }

    fn sub(&self, rhs: &Self) -> Self {
        *self - *rhs
    }

    fn div(&self, rhs: &u16) -> Self {
        *self / *rhs as u32
    }
}

impl AverageMut<u16> for Duration {
    fn add_assign(&mut self, rhs: &Self) {
        *self += *rhs
    }

    fn sub_assign(&mut self, rhs: &Self) {
        *self -= *rhs
    }
}

// It'd be nice if I could make this generic over types that implemented Deref<Target=[T]>, but
// Rust says no (at least until sealed traits (or maybe negative constraints) are a thing?).

/// Implement [Average] for container types.
///
/// if implementing for a generic type, the word 'generic' is added in first, and the type
/// parameter must be `T`. If not generic, the inner type is given after the implementing type.
/// If implementing on a type where it's returning another type (ex: for slices, returning a Vec),
/// add that the last argument.
///
/// Examples:
/// ```
/// average_container!(generic Vec<T>);
/// average_container!(generic [T], Vec[T]);
/// average_container!(Bytes, u8);
/// ```
macro_rules! average_container {
    ($typ:ty, $inner_typ:ty, $return_typ:ty) => {
        impl<Div> Average<Div, $return_typ> for $typ
        where
            Div: convert::Into<$inner_typ> + Copy
        {
            fn add(&self, other: &Self) -> $return_typ {
                assert!(self.len() == other.len(), "The two collections must be the same length to average");
                self.iter().zip(other.iter()).map(|(lhs, rhs)| *lhs + *rhs).collect()
            }
            fn sub(&self, other: &Self) -> $return_typ {
                assert!(self.len() == other.len(), "The two collections must be the same length to average");
                self.iter().zip(other.iter()).map(|(lhs, rhs)| *lhs - *rhs).collect()
            }
            fn div(&self, rhs: &Div) -> $return_typ {
                let divisor: $inner_typ = Into::<$inner_typ>::into(*rhs);
                self.iter().map(|lhs| *lhs / divisor).collect()
            }
        }
    };
    ($typ:ty, $inner_typ:ty) => {
        average_container!($typ, $inner_typ, Self);
    };
    (generic $typ:ty, $return_typ:ty) => {
        impl<T, Div> Average<Div, $return_typ> for $typ
        where
            T: Primitive + ops::AddAssign + ops::SubAssign,
            Div: convert::Into<T> + Copy
        {
            fn add(&self, other: &Self) -> $return_typ {
                assert!(self.len() == other.len(), "The two collections must be the same length to average");
                self.iter().zip(other.iter()).map(|(lhs, rhs)| *lhs + *rhs).collect()
            }
            fn sub(&self, other: &Self) -> $return_typ {
                assert!(self.len() == other.len(), "The two collections must be the same length to average");
                self.iter().zip(other.iter()).map(|(lhs, rhs)| *lhs - *rhs).collect()
            }
            fn div(&self, rhs: &Div) -> $return_typ {
                let divisor: T = Into::<T>::into(*rhs);
                self.iter().map(|lhs| *lhs / divisor).collect()
            }
        }
    };
    (generic $typ:ty) => {
        average_container!(generic $typ, Self);
    };
}

/// Like [average_container], but for [AverageMut]. Arguments are handled exactly the same way.
macro_rules! average_mut_container {
    ($typ:ty, $inner_typ:ty, $return_typ:ty) => {
        impl<Div> AverageMut<Div, $return_typ> for $typ
        where
            Div: convert::Into<$inner_typ> + Copy
        {
            fn add_assign(&mut self, other: &Self) {
                assert!(self.len() == other.len(), "The two collections must be the same length to average");
                self.iter_mut().zip(other.iter()).for_each(|(lhs, rhs)| *lhs += *rhs)
            }

            fn sub_assign(&mut self, other: &Self) {
                assert!(self.len() == other.len(), "The two collections must be the same length to average");
                self.iter_mut().zip(other.iter()).for_each(|(lhs, rhs)| *lhs -= *rhs)
            }
        }
    };
    ($typ:ty, $inner_typ:ty) => {
        average_mut_container!($typ, $inner_typ, Self);
    };
    (generic $typ:ty, $return_typ:ty) => {
        impl<T, Div> AverageMut<Div, $return_typ> for $typ
        where
            T: Primitive + ops::AddAssign + ops::SubAssign,
            Div: convert::Into<T> + Copy
        {
            fn add_assign(&mut self, other: &Self) {
                self.iter_mut().zip(other.iter()).for_each(|(lhs, rhs)| *lhs += *rhs)
            }

            fn sub_assign(&mut self, other: &Self) {
                self.iter_mut().zip(other.iter()).for_each(|(lhs, rhs)| *lhs -= *rhs)
            }
        }
    };
    (generic $typ:ty) => {
        average_mut_container!(generic $typ, Self);
    };
}

average_container!(generic Vec<T>);
average_mut_container!(generic Vec<T>);
average_container!(generic[T], Vec<T>);
average_mut_container!(generic[T], Vec<T>);
average_container!(Bytes, u8);
average_container!(BytesMut, u8);
average_mut_container!(BytesMut, u8);

impl<Div, Px, Co> Average<Div> for ImageBuffer<Px, Co>
where
    Px: Pixel + 'static,
    <Px as Pixel>::Subpixel: 'static,
    Co: ops::Deref<Target = [Px::Subpixel]> + Average<Div>,
{
    fn add(&self, rhs: &Self) -> Self {
        let new_raw = self.as_raw().add(rhs.as_raw());
        ImageBuffer::from_raw(self.width(), self.height(), new_raw)
            .expect("An identically sized Vec to work as an image")
    }

    fn sub(&self, rhs: &Self) -> Self {
        let new_raw = self.as_raw().sub(rhs.as_raw());
        ImageBuffer::from_raw(self.width(), self.height(), new_raw)
            .expect("An identically sized Vec to work as an image")
    }

    fn div(&self, rhs: &Div) -> Self {
        let new_raw = self.as_raw().div(rhs);
        ImageBuffer::from_raw(self.width(), self.height(), new_raw)
            .expect("An identically sized Vec to work as an image")
    }
}

pub trait MovingAverage<T> {
    /// Add a new sample and return the new moving average afterwards.
    fn update(&mut self, new_value: T) -> T {
        self.push(new_value);
        self.current_value()
            .expect("There to be a value as we just pushed one")
    }

    /// Add a new sample for the moving average.
    fn push(&mut self, new_value: T);

    /// Get the current moving average. If there have been no samples yet, it returns [None]
    fn current_value(&self) -> Option<T>;
}

/// A moving average where all samples are weighted identically.
#[derive(Clone, Debug)]
pub struct BoxcarFilter<T, const N: usize> {
    frames: Arc<RwLock<VecDeque<T>>>,
    // Possibly a premature optimization
    sums: Arc<RwLock<Option<T>>>,
}

impl<T, const N: usize> BoxcarFilter<T, N> {
    pub fn new() -> Self {
        Self {
            frames: Arc::new(RwLock::new(VecDeque::with_capacity(N))),
            sums: Arc::new(RwLock::new(None)),
        }
    }
}

impl<T, const N: usize> Default for BoxcarFilter<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> MovingAverage<T> for BoxcarFilter<T, N>
where
    T: AverageMut<u16> + Clone,
{
    fn push(&mut self, new_value: T) {
        // Hold write locks for this entire method
        let mut frames = self.frames.write().unwrap();
        let mut sums = self.sums.write().unwrap();
        // Always check to see if we need to pop first to keep the queue from getting too big
        if frames.len() >= N {
            if let Some(old_frame) = frames.pop_front() {
                if let Some(sums) = sums.deref_mut() {
                    sums.sub_assign(&old_frame);
                }
            }
        }
        match sums.deref_mut() {
            Some(sums) => sums.add_assign(&new_value),
            None => {
                sums.replace(new_value.clone());
            }
        }
        frames.push_back(new_value);
    }

    fn current_value(&self) -> Option<T> {
        // Always frames, then sums.
        let frames = self.frames.read().unwrap();
        let sums = self.sums.read().unwrap();
        let num_frames = frames.len() as u16;
        sums.as_ref().map(|sums| sums.clone().div(&num_frames))
    }
}

impl<T, const N: usize> PartialEq for BoxcarFilter<T, N>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        let left = self.frames.read().unwrap();
        let right = other.frames.read().unwrap();
        left.deref() == right.deref()
    }
}

impl<T, const N: usize> Eq for BoxcarFilter<T, N> where T: Eq {}
