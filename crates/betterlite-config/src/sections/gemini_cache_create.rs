use serde::Deserialize;

use crate::env_integer::get_env_int_or_none;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const MINIMUM_ENVIRONMENT_VARIABLE: &str = "MINIMUM_PROMPT_CACHE_TOKEN_COUNT";

// LiteLLM reads the minimum from the environment alone, so this view reads no key; the skipped field keeps serde's per-key ignore path while the struct stays non-empty
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    pub view: (),
}

// LiteLLM reads the variable once at import, and an unusable value leaves the per-model minimum in place
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub minimum_token_override: Option<u64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured =
            get_env_int_or_none(lookup.lookup(MINIMUM_ENVIRONMENT_VARIABLE).as_deref());
        Ok(Settings {
            // Python's int holds a negative threshold too, and no count is ever below zero
            minimum_token_override: configured.map(|value| u64::try_from(value).unwrap_or(0)),
        })
    }
}
