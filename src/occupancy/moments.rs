// SPDX-License-Identifier: GPL-3.0-or-later
use std::ops::Index;

use super::point::PointTemperature;

// TODO: figure out a better/cleaner way to express these
const MOMENT_ORDERS: [(i32, i32); 10] = [
    // 0-order
    (0, 0),
    // 1-order
    (0, 1),
    (1, 0),
    // 2-order
    (0, 2),
    (1, 1),
    (2, 0),
    // 3-order
    (0, 3),
    (1, 2),
    (2, 1),
    (3, 0),
];

const fn tuple_to_index(order_tuple: (i32, i32)) -> usize {
    // TODO: once const_panic is released (1.58 or 9 I think?) this can be uncommented
    //assert!(order_tuple.0 >= 0 && order_tuple.1 >= 0, "Each order_tuple must be greater than 0");
    match order_tuple.0 + order_tuple.1 {
        0 => 0,
        sum @ 1..=3 => {
            let order_base_index = match sum {
                1 => 1,
                2 => 3,
                3 => 6,
                _ => {
                    // Again, this can be fixed to just be unreachable!() in a later version of
                    // Rust.
                    //unreachable!()
                    usize::MAX
                }
            };
            order_base_index + order_tuple.0 as usize
        }
        _ => {
            // Ditto here
            //panic!("Only moments up to order 3 are supported.")
            usize::MAX
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[repr(transparent)]
pub(super) struct RawMoments([f32; 10]);

impl RawMoments {
    fn new(point_temperatures: &[PointTemperature]) -> Self {
        let mut raw_moments = [0f32; 10];
        MOMENT_ORDERS
            .iter()
            .zip(raw_moments.iter_mut())
            .for_each(|((i, j), raw_moment)| {
                *raw_moment = point_temperatures
                    .iter()
                    .map(|(point, temperature)| {
                        let x = point.x as f32;
                        let y = point.y as f32;
                        x.powi(*i) * y.powi(*j) * temperature
                    })
                    .sum();
            });
        Self(raw_moments)
    }

    fn centroid(&self) -> (f32, f32) {
        let zeroth = self[(0, 0)];
        (
            // centroid of x is M10 / M00
            self[(1, 0)] / zeroth,
            // centroid of y is M01 / M00
            self[(0, 1)] / zeroth,
        )
    }
}

impl Index<(i32, i32)> for RawMoments {
    type Output = f32;

    fn index(&self, index: (i32, i32)) -> &Self::Output {
        let index = tuple_to_index(index);
        &self.0[index]
    }
}

#[derive(Clone, Debug, PartialEq)]
#[repr(transparent)]
pub(super) struct CentralMoments([f32; 8]);

impl CentralMoments {
    fn new(raw_moments: &RawMoments) -> Self {
        let centroid = raw_moments.centroid();
        let centroid_2 = (centroid.0.powi(2), centroid.1.powi(2));
        // Using the definitions from Wikipedia (but the order of orders is different!)
        let central_moments: [f32; 8] = [
            // 00 is defined as M00
            raw_moments[(0, 0)],
            // 01 is defined as 0
            // 10 is also defined as 0
            // 02
            raw_moments[(0, 2)] - centroid.1 * raw_moments[(0, 1)],
            // 11
            raw_moments[(1, 0)] - centroid.0 * raw_moments[(0, 1)],
            // 20
            raw_moments[(2, 0)] - centroid.0 * raw_moments[(1, 0)],
            // 03
            raw_moments[(0, 3)] - 3.0 * centroid.1 * raw_moments[(0, 2)]
                + 2.0 * centroid_2.1 * raw_moments[(0, 1)],
            // 12
            raw_moments[(1, 2)]
                - 2.0 * centroid.1 * raw_moments[(1, 1)]
                - centroid.0 * raw_moments[(0, 2)]
                + 2.0 * centroid_2.1 * raw_moments[(1, 0)],
            // 21
            raw_moments[(2, 1)]
                - 2.0 * centroid.0 * raw_moments[(1, 1)]
                - centroid.1 * raw_moments[(2, 0)]
                + 2.0 * centroid_2.0 * raw_moments[(0, 1)],
            // 30
            raw_moments[(3, 0)] - 3.0 * centroid.0 * raw_moments[(2, 0)]
                + 2.0 * centroid_2.0 * raw_moments[(1, 0)],
        ];
        Self(central_moments)
    }
}

impl Index<(i32, i32)> for CentralMoments {
    type Output = f32;

    fn index(&self, index: (i32, i32)) -> &Self::Output {
        match index {
            // 00 is mapped normally
            (0, 0) => &self.0[0],
            // 01 and 10 are defined as 0
            (1, 0) | (0, 1) => &0.0,
            index => {
                // Account for 01 and 10
                let index = tuple_to_index(index) - 2;
                &self.0[index]
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[repr(transparent)]
pub(super) struct ScaleInvariantMoments([f32; 7]);

impl ScaleInvariantMoments {
    fn new(central_moments: &CentralMoments) -> Self {
        let mut scale_invariants = [0f32; 7];

        MOMENT_ORDERS
            .iter()
            // Skipping the first three, as i and j need to sum to >= 2
            .skip(3)
            .zip(
                // Skip 1 to skip 00
                central_moments.0.iter().skip(1),
            )
            .zip(scale_invariants.iter_mut())
            .for_each(|(((i, j), central_moment), scaled_moment)| {
                let power = 1.0 + ((i + j) as f32 / 2.0);
                let denom = central_moments[(0, 0)].powf(power);
                *scaled_moment = central_moment / denom
            });
        Self(scale_invariants)
    }
}

impl Index<(i32, i32)> for ScaleInvariantMoments {
    type Output = f32;

    fn index(&self, index: (i32, i32)) -> &Self::Output {
        assert!(index.0 + index.1 >= 2, "The sum of i and j must be >= 2");
        // Account for skipping 00, 01, and 10.
        let index = tuple_to_index(index) - 3;
        &self.0[index]
    }
}

pub(super) fn hu_moments(point_temperatures: &[PointTemperature]) -> [f32; 7] {
    let raw_moments = RawMoments::new(point_temperatures);
    let central_moments = CentralMoments::new(&raw_moments);
    let n = ScaleInvariantMoments::new(&central_moments);
    // The Hu moments are built up from only a couple of terms. To optimize things a little I'm
    // computing those terms first (as well as a few powers of some of those terms), then
    // calculating the actual Hu moments.
    // n20 - n02, used in h2 and h6
    let diff_20_02 = n[(2, 0)] - n[(0, 2)];
    // n30 + n12, used in h4, h5, h6, and h7
    let sum_30_12 = n[(3, 0)] + n[(1, 2)];
    let sum_30_12_2 = sum_30_12.powi(2);
    // n21 + n03, used in h4, h5, h6, and h7
    let sum_21_03 = n[(2, 1)] + n[(0, 3)];
    let sum_21_03_2 = sum_21_03.powi(2);
    // The next two values are used in h3, h5, and h7
    // 3 * n21 - n03
    let diff_3_21_03 = 3.0 * n[(2, 1)] - n[(0, 3)];
    // n30 - 3 * n12
    let diff_30_3_12 = n[(3, 0)] - 3.0 * n[(1, 2)];
    // This is the last portion of h5 and h7
    let tail_5_7 = sum_21_03 * (3.0 * sum_30_12_2 - sum_21_03_2);
    [
        // h1
        n[(2, 0)] + n[(0, 2)],
        // h2
        diff_20_02.powi(2) + 4.0 * n[(1, 1)].powi(2),
        // h3
        diff_30_3_12.powi(2) + diff_3_21_03.powi(2),
        // h4
        sum_30_12_2 + sum_21_03_2,
        // h5
        diff_30_3_12 * sum_30_12 * (sum_30_12_2 - 3.0 * sum_21_03_2) + diff_3_21_03 * tail_5_7,
        // h6
        diff_20_02 * (sum_30_12_2 - sum_21_03_2) + 4.0 * n[(1, 1)] * sum_30_12 * sum_21_03,
        // h7
        diff_3_21_03 * sum_30_12 * (sum_30_12_2 - 3.0 * sum_21_03_2) - diff_30_3_12 * tail_5_7,
    ]
}
