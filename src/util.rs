// SPDX-License-Identifier: GPL-3.0-or-later
use std::panic;

use num_traits::Num;
use tokio::task::JoinError;

/// Parse an unsigned integer from a base-10 or base-16 string representation.
///
/// If the string starts with `0x`, the rest of the string is treated as a hexadecimal integer.
/// Otherwise the string is treated as a decimal integer.
#[allow(clippy::from_str_radix_10)]
pub fn parse_int_decimal_hex<U: Num>(num_str: &str) -> Result<U, <U as Num>::FromStrRadixErr> {
    let num_str = num_str.to_ascii_lowercase();
    if let Some(hex_str) = num_str.strip_prefix("0x") {
        U::from_str_radix(hex_str, 16)
    } else {
        U::from_str_radix(num_str.as_str(), 10)
    }
}

pub(crate) fn flatten_join_result<T, E>(
    join_result: Result<Result<T, E>, JoinError>,
) -> anyhow::Result<T>
where
    anyhow::Error: From<E>,
{
    match join_result {
        Ok(inner_result) => Ok(inner_result?),
        Err(join_error) => {
            if join_error.is_panic() {
                panic::resume_unwind(join_error.into_panic());
            } else {
                Err(join_error.into())
            }
        }
    }
}
