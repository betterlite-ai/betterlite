use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;
use serde::Deserialize;
const MAX_FILE_SIZE_PATH: &str = "general_settings.max_file_size_mb";
const MAX_BATCH_FILE_SIZE_PATH: &str = "general_settings.max_batch_file_size_mb";
const ALLOWED_EXTENSIONS_PATH: &str = "general_settings.allowed_file_extensions";
const BLOCKED_EXTENSIONS_PATH: &str = "general_settings.blocked_file_extensions";
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}
#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    max_file_size_mb: Option<JsonValue>,
    max_batch_file_size_mb: Option<JsonValue>,
    allowed_file_extensions: Option<JsonValue>,
    blocked_file_extensions: Option<JsonValue>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub max_file_size_mb: Option<i64>,
    pub max_batch_file_size_mb: Option<i64>,
    pub allowed_file_extensions: Option<Vec<String>>,
    pub blocked_file_extensions: Option<Vec<String>>,
}
impl Section for Raw {
    type Settings = Settings;
    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self.general_settings;
        let max_file = raw.max_file_size_mb.as_ref();
        let max_batch = raw.max_batch_file_size_mb.as_ref();
        let allowed = raw.allowed_file_extensions.as_ref();
        let blocked = raw.blocked_file_extensions.as_ref();
        Ok(Settings {
            max_file_size_mb: strict_integer(max_file, MAX_FILE_SIZE_PATH)?,
            max_batch_file_size_mb: lax_integer(max_batch, MAX_BATCH_FILE_SIZE_PATH)?,
            allowed_file_extensions: string_list(allowed, ALLOWED_EXTENSIONS_PATH)?,
            blocked_file_extensions: string_list(blocked, BLOCKED_EXTENSIONS_PATH)?,
        })
    }
}
fn strict_integer(raw: Option<&JsonValue>, path: &str) -> Result<Option<i64>, ConfigError> {
    let Some(value) = raw else {
        return Ok(None);
    };
    match value.as_i64() {
        Some(integer) => Ok(Some(integer)),
        None => Err(invalid(path, value)),
    }
}
fn lax_integer(raw: Option<&JsonValue>, path: &str) -> Result<Option<i64>, ConfigError> {
    let Some(value) = raw else {
        return Ok(None);
    };
    let coerced = match value {
        JsonValue::Null => Some(None),
        JsonValue::Bool(flag) => Some(Some(i64::from(*flag))),
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| {
                number
                    .as_f64()
                    .and_then(|float| float.to_string().parse::<i64>().ok())
            })
            .map(Some),
        JsonValue::String(text) => lax_string_integer(text).map(Some),
        JsonValue::Array(_) | JsonValue::Object(_) => None,
    };
    coerced.ok_or_else(|| invalid(path, value))
}
fn lax_string_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if let Ok(value) = trimmed.parse::<i64>() {
        return Some(value);
    }
    let (integer, fraction) = trimmed.split_once('.')?;
    if fraction.is_empty() || !fraction.bytes().all(|digit| digit == b'0') {
        return None;
    }
    integer.parse::<i64>().ok()
}
fn strings_of(value: &JsonValue) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}
fn string_list(raw: Option<&JsonValue>, path: &str) -> Result<Option<Vec<String>>, ConfigError> {
    let Some(value) = raw else { return Ok(None) };
    strings_of(value)
        .map(Some)
        .ok_or_else(|| invalid(path, value))
}
fn invalid(path: &str, value: &JsonValue) -> ConfigError {
    let flag = match value {
        JsonValue::Bool(flag) => *flag,
        JsonValue::Null
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => false,
    };
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: flag,
    }
}
mod tests;
