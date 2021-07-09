// SPDX-License-Identifier: GPL-3.0-or-later
use num_traits::Num;

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
