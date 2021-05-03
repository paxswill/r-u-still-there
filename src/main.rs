// SPDX-License-Identifier: GPL-3.0-or-later
use std::fs;
use std::path::Path;

#[macro_use]
extern crate lazy_static;

mod error;
mod image_buffer;
mod moving_average;
mod pipeline;
mod render;
mod settings;
mod spmc;
mod stream;

use crate::pipeline::Pipeline;
use crate::settings::Settings;

#[tokio::main]
async fn main() {
    // Static config location (and relative!) for now
    let config_data = fs::read(Path::new("./config.toml")).unwrap();
    let config: Settings = toml::from_slice(&config_data).unwrap();

    let app = Pipeline::from(config);

    app.await;
}
