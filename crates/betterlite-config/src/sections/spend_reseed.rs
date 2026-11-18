use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// litellm.constants.SPEND_COUNTER_RESEED_LOCKS_MAX_SIZE, read with int(os.getenv(...)) at import
const LOCKS_MAX_SIZE_VARIABLE: &str = "SPEND_COUNTER_RESEED_LOCKS_MAX_SIZE";
const DEFAULT_LOCKS_MAX_SIZE: i64 = 10_000;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // a skipped marker keeps this environment-only section a YAML mapping
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    // negative and zero are legitimate: the registry then evicts every lock it inserts
    pub locks_max_size: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            locks_max_size: DEFAULT_LOCKS_MAX_SIZE,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let locks_max_size = match lookup.lookup(LOCKS_MAX_SIZE_VARIABLE) {
            None => DEFAULT_LOCKS_MAX_SIZE,
            Some(text) => locks_max_size_from_text(&text)?,
        };
        Ok(Settings { locks_max_size })
    }
}

// Python's int() strips surrounding whitespace and raises on an empty or malformed value at import
fn locks_max_size_from_text(text: &str) -> Result<i64, ConfigError> {
    let path = LOCKS_MAX_SIZE_VARIABLE.to_owned();
    text.trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger { path, source })
}
