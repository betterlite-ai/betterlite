use betterlite_db::database_reencryption::{PREVIOUS_KEY_VARIABLE, SALT_KEY_ENV_VAR};
use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const RECURSE_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";
pub const DEFAULT_RECURSE_DEPTH: i64 = 100;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _marker: (),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub max_recurse_depth: i64,
    pub migrate_from_master_key: Option<SecretString>,
    pub salt_key_is_set: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_recurse_depth: DEFAULT_RECURSE_DEPTH,
            migrate_from_master_key: None,
            salt_key_is_set: false,
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
            max_recurse_depth: depth_from_env(lookup)?,
            migrate_from_master_key: lookup.lookup(PREVIOUS_KEY_VARIABLE).map(SecretString::from),
            salt_key_is_set: lookup.lookup(SALT_KEY_ENV_VAR).is_some(),
        })
    }
}

// LiteLLM calls int() once at import: a bad or empty value stops startup instead of taking the default
fn depth_from_env<L: EnvironmentLookup>(lookup: &L) -> Result<i64, ConfigError> {
    let Some(text) = lookup.lookup(RECURSE_DEPTH_VARIABLE) else {
        return Ok(DEFAULT_RECURSE_DEPTH);
    };
    betterlite_env::number::<i64>(RECURSE_DEPTH_VARIABLE, &text)?.ok_or_else(|| {
        ConfigError::from(betterlite_env::EnvError::Number {
            name: RECURSE_DEPTH_VARIABLE.to_owned(),
        })
    })
}
