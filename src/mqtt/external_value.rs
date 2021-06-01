// SPDX-License-Identifier: GPL-3.0-or-later
use serde::Deserialize;
use tracing::debug;

use std::convert::TryFrom;
use std::path::PathBuf;
/// A type that can either be deserialized either from a string or a path to a file.
///
/// If just a plain string is present, that value is used. If a map with a key 'file' with a string
/// value is provided, the inner string value is taken as a path to a file, the contents of which
/// will be read and used ad the final value.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(try_from = "InnerExternalValue")]
pub struct ExternalValue(String);

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum InnerExternalValue {
    File { file: PathBuf },

    String(String),
}

impl From<ExternalValue> for String {
    fn from(value: ExternalValue) -> Self {
        value.0
    }
}

impl TryFrom<InnerExternalValue> for ExternalValue {
    type Error = std::io::Error;

    fn try_from(inner: InnerExternalValue) -> std::io::Result<Self> {
        match inner {
            InnerExternalValue::File { file } => {
                debug!("Reading secret from {:?}", file);
                std::fs::read_to_string(file).map(Self)
            }
            InnerExternalValue::String(s) => {
                debug!("Using secret directly");
                Ok(Self(s))
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::ExternalValue;
    use serde::Deserialize;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Wrapper {
        field: ExternalValue,
    }

    #[test]
    fn plain_string() {
        let parsed: Result<Wrapper, _> = toml::from_str(
            r#"
        field = "foo"
        "#,
        );
        assert!(
            parsed.is_ok(),
            "unable to parse a plain string: {:?}",
            parsed
        );
        let parsed = parsed.unwrap();
        assert_eq!(parsed.field.0, "foo".to_string());
    }

    #[test]
    fn missing_file() {
        let parsed: Result<Wrapper, _> = toml::from_str(
            r#"
        field = { file = "/not/a/real/path/foo/bar" }
        "#,
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn read_file() {
        let mut file = NamedTempFile::new().expect("to be able to create a temp file");
        let file_value = "foo bar baz";
        write!(file, "{}", file_value).expect("to be able to write to a new temp file");
        let data = format!(
            r#"
        field = {{ file = "{}" }}
        "#,
            file.path().to_string_lossy()
        );
        let parsed: Result<Wrapper, _> = toml::from_str(&data);
        assert!(parsed.is_ok());
        let parsed = parsed.unwrap();
        assert_eq!(parsed.field.0, file_value.to_string());
    }
}
