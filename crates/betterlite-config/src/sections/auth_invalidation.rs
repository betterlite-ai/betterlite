use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const ACCESS_TTL_VARIABLE: &str = "DEFAULT_ACCESS_GROUP_CACHE_TTL";
const DEFAULT_ACCESS_TTL_SECONDS: f64 = 600.0_f64;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (), // the section reads only the environment, so it owns no YAML key
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub access_group_ttl: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            access_group_ttl: DEFAULT_ACCESS_TTL_SECONDS,
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
            access_group_ttl: access_group_ttl(lookup)?,
        })
    }
}

// LiteLLM parses this at import: an empty or non-integer value stops startup, and zero or negative is used as given
fn access_group_ttl<L>(lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(ACCESS_TTL_VARIABLE) else {
        return Ok(DEFAULT_ACCESS_TTL_SECONDS);
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::Environment {
            source: betterlite_env::EnvError::Number {
                name: ACCESS_TTL_VARIABLE.to_owned(),
            },
        });
    }
    let value = trimmed
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: ACCESS_TTL_VARIABLE.to_owned(),
            source,
        })?;
    Ok(Decimal::from(value).to_f64().unwrap_or_default())
}
