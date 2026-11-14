use std::str::FromStr;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const QUEUE_MAXSIZE_ENV: &str = "ANTHROPIC_MESSAGES_STREAM_RELAY_QUEUE_MAXSIZE";
pub const MAX_DETACHED_STREAM_DRAINS_ENV: &str = "ANTHROPIC_MESSAGES_MAX_DETACHED_STREAM_DRAINS";
pub const DEFAULT_QUEUE_MAXSIZE: i64 = 1024;
pub const DEFAULT_MAX_DETACHED_STREAM_DRAINS: i64 = 100;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

// Both values keep LiteLLM's any-integer range; the relay rejects a non-positive queue and reads
// zero or negative detached-drain caps as "no draining".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub queue_maxsize: i64,
    pub max_detached_stream_drains: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            queue_maxsize: DEFAULT_QUEUE_MAXSIZE,
            max_detached_stream_drains: DEFAULT_MAX_DETACHED_STREAM_DRAINS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            queue_maxsize: environment_integer(lookup, QUEUE_MAXSIZE_ENV)?
                .unwrap_or(DEFAULT_QUEUE_MAXSIZE),
            max_detached_stream_drains: environment_integer(
                lookup,
                MAX_DETACHED_STREAM_DRAINS_ENV,
            )?
            .unwrap_or(DEFAULT_MAX_DETACHED_STREAM_DRAINS),
        })
    }
}

fn environment_integer<L>(lookup: &L, name: &str) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name).filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    match python_integer(&text) {
        Some(value) => Ok(Some(value)),
        None => Err(ConfigError::Environment {
            source: betterlite_env::EnvError::Number {
                name: name.to_owned(),
            },
        }),
    }
}

// Python's int() takes surrounding whitespace, one sign and single underscores between digits
fn python_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    i64::from_str(&trimmed.replace('_', "")).ok()
}
