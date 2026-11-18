use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const CONCURRENCY_VARIABLE: &str = "PROXY_DB_LOOKUP_MAX_CONCURRENCY";
const DEADLINE_VARIABLE: &str = "PROXY_DB_LOOKUP_DEADLINE_SECONDS";
const DEFAULT_CONCURRENCY: usize = 25;
const DEFAULT_DEADLINE_SECONDS: f64 = 10.0;
const MIN_CONCURRENCY: usize = 1;
const MIN_DEADLINE_SECONDS: f64 = 0.1;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // a skipped marker keeps this environment-only section a YAML mapping
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub max_concurrency: usize,
    pub deadline_seconds: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_concurrency: DEFAULT_CONCURRENCY,
            deadline_seconds: DEFAULT_DEADLINE_SECONDS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let max_concurrency = match lookup.lookup(CONCURRENCY_VARIABLE) {
            None => DEFAULT_CONCURRENCY,
            Some(text) => concurrency_from_text(&text)?,
        };
        let deadline_seconds = match lookup.lookup(DEADLINE_VARIABLE) {
            None => DEFAULT_DEADLINE_SECONDS,
            Some(text) => deadline_from_text(&text)?,
        };
        Ok(Settings {
            max_concurrency,
            deadline_seconds,
        })
    }
}

fn concurrency_from_text(text: &str) -> Result<usize, ConfigError> {
    let parsed = text
        .trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: CONCURRENCY_VARIABLE.to_owned(),
            source,
        })?;
    Ok(usize::try_from(parsed)
        .unwrap_or_default()
        .max(MIN_CONCURRENCY))
}

fn deadline_from_text(text: &str) -> Result<f64, ConfigError> {
    let parsed = text
        .trim()
        .parse::<f64>()
        .map_err(|source| ConfigError::InvalidFloat {
            path: DEADLINE_VARIABLE.to_owned(),
            source,
        })?;
    Ok(parsed.max(MIN_DEADLINE_SECONDS))
}
