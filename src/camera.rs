// SPDX-License-Identifier: GPL-3.0-or-later
use futures::stream::{Stream, StreamExt};
use ndarray::Array2;
use thermal_camera::ThermalCamera;
use tokio::time::{self, Duration};
use tokio_stream::wrappers::IntervalStream;

use std::error::Error as StdError;

pub fn camera_stream<'a, C: 'static, E>(
    mut camera: C,
    interval_duration: Duration,
) -> impl Stream<Item = Array2<f32>>
where
    E: StdError + 'a,
    C: ThermalCamera<'a, Error = E>,
    C: 'a,
{
    let interval = time::interval(interval_duration);
    let interval_stream = IntervalStream::new(interval);
    interval_stream.map(Box::new(move |_| camera.image().unwrap()))
}
