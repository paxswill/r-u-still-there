// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;

use std::collections::HashMap;
use std::net;

use crate::stream::{mjpeg::MjpegStream, VideoStream};

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(tag = "kind")]
pub enum StreamKind {
    Mjpeg,
}

impl Default for StreamKind {
    fn default() -> Self {
        Self::Mjpeg
    }
}

fn default_address() -> net::IpAddr {
    net::IpAddr::from([127u8, 0u8, 0u8, 1u8])
}

fn default_port() -> u16 {
    9000u16
}

fn default_streams<'a>() -> HashMap<&'a str, StreamKind> {
    [("/stream", StreamKind::default())].iter().cloned().collect()
}

#[derive(Debug, Deserialize)]
pub struct StreamSettings<'a> {
    #[serde(default)]
    pub kind: StreamKind,

    /// The address to bind the server to. Defaults to `127.0.0.1`.
    #[serde(default = "default_address")]
    address: net::IpAddr,

    /// The port to bind the server to. Default to `9000`.
    #[serde(default = "default_port")]
    port: u16,

    /// A mapping of paths to serve different stream types from. Keys are full paths, and values
    /// are variants of [StreamKind]. The default is `/mjpeg` as an MJPEG stream.
    #[serde(borrow, default = "default_streams")]
    pub streams: HashMap<&'a str, StreamKind>,
}

impl From<StreamSettings<'_>> for net::SocketAddr {
    fn from(settings: StreamSettings<'_>) -> net::SocketAddr {
        match settings.address {
            net::IpAddr::V4(ip) => net::SocketAddr::from((ip, settings.port)),
            net::IpAddr::V6(ip) => net::SocketAddr::from((ip, settings.port)),
        }
    }
}