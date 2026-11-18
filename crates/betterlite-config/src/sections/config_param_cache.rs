use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const TTL_VARIABLE: &str = "LITELLM_CONFIG_PARAM_CACHE_TTL_SECONDS";
const DEFAULT_TTL_SECONDS: f64 = 60.0;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // every value is read from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub ttl_seconds: f64,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(text) = lookup.lookup(TTL_VARIABLE) else {
            return Ok(Settings {
                ttl_seconds: DEFAULT_TTL_SECONDS,
            });
        };
        let parsed = betterlite_env::number::<f64>(TTL_VARIABLE, &text)
            .map_err(|source| ConfigError::Environment { source })?;
        Ok(Settings {
            ttl_seconds: parsed.unwrap_or(DEFAULT_TTL_SECONDS),
        })
    }
}
