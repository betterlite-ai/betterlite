use std::time::Duration;

use betterlite_env::number;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const TTL_VARIABLE: &str = "LITELLM_MAX_ITERATIONS_TTL";
pub const DEFAULT_TTL_SECONDS: u64 = 3600;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (), // the section reads only the environment, so it owns no YAML key
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub max_iterations_ttl: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_iterations_ttl: Duration::from_secs(DEFAULT_TTL_SECONDS),
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
            max_iterations_ttl: max_iterations_ttl(lookup.lookup(TTL_VARIABLE).as_deref())?,
        })
    }
}

// int(os.getenv("LITELLM_MAX_ITERATIONS_TTL", 3600)): unset or empty keeps the 3600 second default, and text that is not a count stops the load
pub fn max_iterations_ttl(value: Option<&str>) -> Result<Duration, ConfigError> {
    let Some(text) = value else {
        return Ok(Duration::from_secs(DEFAULT_TTL_SECONDS));
    };
    let Some(seconds) = number::<u64>(TTL_VARIABLE, text)? else {
        return Ok(Duration::from_secs(DEFAULT_TTL_SECONDS));
    };
    Ok(Duration::from_secs(seconds))
}
