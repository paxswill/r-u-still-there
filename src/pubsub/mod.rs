// SPDX-License-Identifier: GPL-3.0-or-later
mod traits;
mod spmc;
mod permit;

pub use traits::{Publisher, Subscriber};
pub use spmc::Sender as Bus;
pub use permit::Permit;