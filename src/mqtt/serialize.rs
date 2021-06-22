// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Serialize;

/// Very similar to [serde_json::to_writer], except that bare strings aren't quoted, bare null
/// values are skipped, and it creates it's own buffer.
pub(super) fn serialize<T>(value: &T) -> serde_json::Result<Vec<u8>>
where
    T: Serialize,
{
    match serde_json::to_value(value)? {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::String(string_val) => Ok(string_val.into_bytes()),
        value => serde_json::to_vec(&value),
    }
}
