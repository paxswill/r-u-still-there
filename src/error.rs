// SPDX-License-Identifier: GPL-3.0-or-later
use tokio::sync::broadcast::error as broadcast_error;
use tokio::task::JoinError;

use std::convert::Infallible;
use std::error::Error as StdError;
use std::fmt;

pub enum Error<BcT> {
    BroadcastSendError(broadcast_error::SendError<BcT>),
    ImpossibleError(Infallible),
    CancelledThread(JoinError),
}

impl<BcT> fmt::Debug for Error<BcT>
where
    BcT: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::BroadcastSendError(e) => f.debug_tuple("BroadcastSendError").field(e).finish(),
            Self::ImpossibleError(_) => f.debug_tuple("ImpossibleError").finish(),
            Self::CancelledThread(e) => f.debug_tuple("CancelledThread").field(e).finish(),
        }
    }
}

impl<BcT> fmt::Display for Error<BcT>
where
    BcT: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::BroadcastSendError(e) => write!(f, "{}", e),
            Self::ImpossibleError(_) => write!(f, "Theoretically impossible error."),
            Self::CancelledThread(e) => write!(f, "{}", e),
        }
    }
}

impl<BcT> StdError for Error<BcT>
where
    BcT: 'static + fmt::Debug + fmt::Display,
{
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::BroadcastSendError(e) => Some(e),
            Self::ImpossibleError(e) => Some(e),
            Self::CancelledThread(e) => Some(e),
        }
    }
}

impl<BcT> From<broadcast_error::SendError<BcT>> for Error<BcT> {
    fn from(e: broadcast_error::SendError<BcT>) -> Self {
        Self::BroadcastSendError(e)
    }
}

impl<BcT> From<Infallible> for Error<BcT> {
    fn from(e: Infallible) -> Self {
        Self::ImpossibleError(e)
    }
}

impl<BcT> From<JoinError> for Error<BcT> {
    fn from(e: JoinError) -> Self {
        // Bubble panics up
        Self::CancelledThread(e.try_into_panic().unwrap_err())
    }
}
