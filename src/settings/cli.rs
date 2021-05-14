// SPDX-License-Identifier: GPL-3.0-or-later
use structopt::StructOpt;

use std::path::PathBuf;

#[derive(Debug, StructOpt)]
#[structopt()]
pub struct Args {
    /// Path to a configuration file.
    #[structopt(short, long, parse(from_os_str), default_value = "config.toml")]
    pub config_path: PathBuf,
}
