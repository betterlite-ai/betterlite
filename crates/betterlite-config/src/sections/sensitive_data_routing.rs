use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// LiteLLM's DEFAULT_SENSITIVE_ROUTING_TTL, used whenever the variable is unset or empty
pub const DEFAULT_TTL_SECONDS: u64 = 3600;
pub const TTL_ENV_VAR: &str = "LITELLM_SENSITIVE_ROUTING_TTL";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // every value is read from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub ttl_seconds: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ttl_seconds: DEFAULT_TTL_SECONDS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(text) = lookup.lookup(TTL_ENV_VAR) else {
            return Ok(Settings::default());
        };
        let ttl_seconds =
            betterlite_env::number::<u64>(TTL_ENV_VAR, &text)?.unwrap_or(DEFAULT_TTL_SECONDS);
        Ok(Settings { ttl_seconds })
    }
}
