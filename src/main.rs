// SPDX-License-Identifier: GPL-3.0-or-later
use structopt::StructOpt;

use std::fs;

#[macro_use]
extern crate lazy_static;

mod error;
mod image_buffer;
mod moving_average;
mod pipeline;
mod pubsub;
mod render;
mod settings;
mod stream;

use crate::pipeline::Pipeline;
use crate::pubsub::spmc;
use crate::settings::{Args, Settings};

#[tokio::main]
async fn main() {
    let clap_app = Args::clap();
    let matches = clap_app.get_matches();
    let config_data = fs::read(matches.value_of("config-path").unwrap()).unwrap();
    let config: Settings = toml::from_slice(&config_data).unwrap();

    let app = Pipeline::from(config);

    app.await;
}
