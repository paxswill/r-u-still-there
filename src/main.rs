// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::anyhow;
use figment::providers::{Env, Format, Toml, Yaml};
use figment::Figment;
use structopt::StructOpt;
use tracing::{debug, debug_span, error, instrument, Instrument};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt as tracing_fmt, EnvFilter, Registry};

use std::path::PathBuf;

#[macro_use]
extern crate lazy_static;

mod camera;
mod image_buffer;
mod moving_average;
mod mqtt;
mod occupancy;
mod pipeline;
mod pubsub;
mod render;
mod settings;
mod stream;

use crate::pipeline::Pipeline;
use crate::pubsub::spmc;
use crate::settings::{Args, Settings};

/// Select a configuration file to use.
///
/// If there's a path present in the provided [Figment], it will be used. Otherwise, one of
/// `config.toml`, or `config.yaml`, will be searched for (in that order) within the
/// `/etc/r-u-still-there/` directory. If no file is found, [None] is returned.
#[instrument(level = "debug", err)]
fn find_config_file(figment: &Figment) -> anyhow::Result<Option<PathBuf>> {
    let given_config_path = figment
        .find_value("config_path")
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    if let Some(given_config_path) = given_config_path {
        debug!(%given_config_path, "using config path from user");
        let path = PathBuf::from(given_config_path);
        if path.exists() {
            return Ok(Some(path));
        } else {
            return Err(anyhow!("Non-existant config file given: {:?}", path));
        }
    }
    let prefix = PathBuf::from("/etc/r-u-still-there");
    let file_names = ["config.toml", "config.yaml"];
    for name in file_names.iter() {
        let path = prefix.join(name);
        debug!("checking for file {:?}", path);
        if path.exists() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

#[instrument(level = "debug", err)]
fn create_config() -> anyhow::Result<Settings> {
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
            return Err(anyhow!("Unknown file extension for file {:?}", config_path));
        }
    }
    complete_figment = complete_figment
        .merge(Env::prefixed("RUSTILLTHERE_"))
        .merge(&args);
    Ok(complete_figment.extract()?)
}

#[tokio::main]
async fn main() {
    // Create an initial logging config, then update it if needed after the full configuration has
    // been merged.
    // NOTE: This will need updating for tracing-subscriber v0.3.0
    let fmt_sub = tracing_fmt::Layer::default().with_thread_names(true);
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .expect("'info' was not recognized as a valid log filter");
    Registry::default().with(fmt_sub).with(env_filter).init();
    let span = debug_span!("setup");
    let config = {
        let _enter = span.enter();
        let config = create_config().expect("Problem generating configuration");
        debug!(?config, "final config");
        config
    };
    let app = Pipeline::new(config).instrument(span).await;
    match app {
        Ok(app) => app.await,
        Err(e) => error!("{:?}", e),
    }
}
