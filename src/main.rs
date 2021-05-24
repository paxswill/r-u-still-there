// SPDX-License-Identifier: GPL-3.0-or-later
use figment::providers::{Env, Format, Toml, Yaml};
use figment::Figment;
use structopt::StructOpt;

use std::path::PathBuf;

#[macro_use]
extern crate lazy_static;

mod error;
mod image_buffer;
mod moving_average;
mod occupancy;
mod pipeline;
mod pubsub;
mod render;
mod settings;
mod stream;

use crate::pipeline::Pipeline;
use crate::pubsub::spmc;
use crate::settings::{Args, Settings};

// TODO: As with many other areas in this program, the error handling leaves something to be
// desired.

/// Select a configuration file to use.
///
/// If there's a path present in the provided [Figment], it will be used. Otherwise, one of
/// `config.toml`, or `config.yaml`, will be searched for (in that order) within the
/// `/etc/r-u-still-there/` directory. If no file is found, [None] is returned.
fn find_config_file(figment: &Figment) -> Result<Option<PathBuf>, String> {
    let given_config_path = figment
        .find_value("config_path")
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    if let Some(given_config_path) = given_config_path {
        let path = PathBuf::from(given_config_path);
        if path.exists() {
            return Ok(Some(path));
        } else {
            return Err(format!("Non-existant config file given: {:?}", path));
        }
    }
    let prefix = PathBuf::from("/etc/r-u-still-there");
    let file_names = ["config.toml", "config.yaml"];
    for name in file_names.iter() {
        let path = prefix.join(name);
        if path.exists() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn create_config() -> Result<Settings, String> {
    // Configuration priority is as follows from least to greatest:
    // Defaults -> Config file -> Environment variable -> CLI flag
    let args = Args::from_args();
    let initial_figment = Figment::new()
        .merge(Env::prefixed("RUSTILLTHERE_"))
        .merge(&args);
    // Find a config file
    let config_path = find_config_file(&initial_figment)?;
    // TODO: Add single location for derfaults, and use them as the initial figment.
    let mut complete_figment = Figment::new();
    if let Some(config_path) = config_path {
        let config_extension = config_path.extension().map(|ext| ext.to_str()).flatten();
        // It'd be nice if I could use pattern matching here, but there's some missing trait
        // implementations in Figment (ex: Provider for Box<dyn Provider>).
        if config_extension == Some("toml") {
            complete_figment = complete_figment.merge(Toml::file(config_path));
        } else if config_extension == Some("yaml") {
            complete_figment = complete_figment.merge(Yaml::file(config_path));
        } else {
            return Err(format!("Unknown file extension for file {:?}", config_path));
        }
    }
    complete_figment = complete_figment
        .merge(Env::prefixed("RUSTILLTHERE_"))
        .merge(&args);
    complete_figment.extract().map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() {
    let config = create_config().expect("Problem generating configuration");
    println!("\n\nFinal config:\n{:?}", config);
    let app = Pipeline::from(config);
    app.await;
}
