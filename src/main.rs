// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::anyhow;
use structopt::StructOpt;
use tracing::{debug, debug_span, error, instrument, trace, warn, Instrument};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt as tracing_fmt, EnvFilter, Registry};

use std::env;
use std::fs::read_to_string;
use std::path::PathBuf;

mod camera;
mod image_buffer;
mod mqtt;
mod occupancy;
mod pipeline;
mod pubsub;
mod render;
mod settings;
mod stream;
mod temperature;
mod util;

use crate::pipeline::Pipeline;
use crate::pubsub::spmc;
use crate::settings::{Args, Settings};

/// Select a configuration file to use.
///
/// If there's a path present in the provided [Args], it will be used. Otherwise, `config.toml`
/// will be searched for within the configuration directory. If a `CONFIGURATION_DIRECTORY`
/// environment variable exists (ex: systemd sets it in some cases), that is searched. If that
/// variable isn't set, `/etc/r-u-still-there/` is used. If no config file is found, `Ok(None)` is
/// returned.
#[instrument(level = "debug", err)]
fn find_config_file(args: &Args) -> anyhow::Result<Option<PathBuf>> {
    if let Some(cli_config_path) = &args.config_path {
        debug!(?cli_config_path, "using config path from CLI argument");
        let path = cli_config_path.clone();
        if path.exists() {
            return Ok(Some(path));
        } else {
            return Err(anyhow!("Non-existant config file given: {:?}", path));
        }
    }
    // Check for $CONFIGURATION_DIRECTORY, which can be set by systemd. Otherwise use
    // /etc/r-u-still-there
    let prefix = env::var("CONFIGURATION_DIRECTORY")
        .map_or(PathBuf::from("/etc/r-u-still-there"), PathBuf::from);

    // Only supporting TOML
    let path = prefix.join("config.toml");
    debug!("checking for file {:?}", path);
    if path.exists() {
        return Ok(Some(path));
    }
    Ok(None)
}

/// Find and create the final configuration for the application.
#[instrument(level = "debug", err)]
fn create_config() -> anyhow::Result<Settings> {
    // Configuration priority is as follows from least to greatest:
    // Defaults -> Config file -> CLI flag
    let args = Args::from_args();
    // Find a config file
    let config_data = if let Some(path) = find_config_file(&args)? {
        read_to_string(path)?
    } else {
        "".to_string()
    };
    args.apply_to_config_str(&config_data)
}

// Just picking values for these.
#[repr(i32)]
enum ExitCode {
    /// Successful exit code.
    Success = 0,

    /// Exit code for errors not covered by other codes.
    Other = 1,

    /// Exit code for errors originating with the configuration.
    Config = 5,

    /// Exit code for errors originating from the setup process, before the application has
    /// completely started.
    Setup = 10,
}

impl Default for ExitCode {
    fn default() -> Self {
        Self::Success
    }
}

async fn inner_main() -> ExitCode {
    // Create an initial logging config, then update it if needed after the full configuration has
    // been merged.
    let fmt_sub = tracing_fmt::Layer::default()
        .with_thread_names(true)
        .with_ansi(atty::is(atty::Stream::Stdout));
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .expect("'info' was not recognized as a valid log filter");
    let log_format = env::var("RUSTILLTHERE_LOG_FORMAT")
        .unwrap_or_else(|_| "full".to_string())
        .to_ascii_lowercase();
    match log_format.as_str() {
        "json" => {
            Registry::default()
                .with(fmt_sub.json())
                .with(env_filter)
                .init();
        }
        "pretty" => {
            Registry::default()
                .with(fmt_sub.pretty())
                .with(env_filter)
                .init();
        }
        "compact" => {
            Registry::default()
                .with(fmt_sub.compact())
                .with(env_filter)
                .init();
        }
        "full" => {
            Registry::default().with(fmt_sub).with(env_filter).init();
        }
        _ => {
            // If an unknown log format is given, use the default ("full") while also printing a
            // warning once the logger is set up.
            Registry::default().with(fmt_sub).with(env_filter).init();
            warn!(
                "Unknown log format '{}' (must be one of 'json', 'pretty', 'compact', or 'full' (default)",
                log_format
            );
        }
    }
    let span = debug_span!("setup");
    let config = {
        let _enter = span.enter();
        match create_config() {
            Err(err) => {
                trace!("Full error chain: {:#?}", err);
                // Walk the error chain, looking for toml errors
                for cause in err.chain() {
                    if let Some(toml_error) = cause.downcast_ref::<toml::de::Error>() {
                        error!("Configuration error: {}", toml_error);
                        return ExitCode::Config;
                    }
                }
                // Not a toml error if we reach here.
                error!("Error combining configuration: {:?}", err);
                return ExitCode::Setup;
            }
            Ok(config) => {
                debug!(?config, "final config");
                config
            }
        }
    };
    let app = match Pipeline::new(config).instrument(span).await {
        Err(err) => {
            error!("Setup error: {:?}", err);
            return ExitCode::Setup;
        }
        Ok(app) => app,
    };
    if let Err(err) = app.await {
        error!("{:?}", err);
        ExitCode::Other
    } else {
        ExitCode::Success
    }
}

#[tokio::main]
async fn main() {
    let exit_code = inner_main().await;
    std::process::exit(exit_code as i32);
}
