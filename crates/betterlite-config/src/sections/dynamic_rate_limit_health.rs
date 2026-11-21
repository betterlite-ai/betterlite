use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const ERROR_THRESHOLD_VARIABLE: &str = "DYNAMIC_RATE_LIMIT_ERROR_THRESHOLD_PER_MINUTE";
pub const DEFAULT_ERROR_THRESHOLD: i64 = 1;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _marker: (),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub error_threshold_per_minute: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            error_threshold_per_minute: DEFAULT_ERROR_THRESHOLD,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            error_threshold_per_minute: threshold_from_env(lookup)?,
        })
    }
}

// LiteLLM calls int() at import, where an empty value raises; the shared env rule counts empty as unset
fn threshold_from_env<L>(lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(ERROR_THRESHOLD_VARIABLE) else {
        return Ok(DEFAULT_ERROR_THRESHOLD);
    };
    Ok(
        betterlite_env::number::<i64>(ERROR_THRESHOLD_VARIABLE, &text)?
            .unwrap_or(DEFAULT_ERROR_THRESHOLD),
    )
}
