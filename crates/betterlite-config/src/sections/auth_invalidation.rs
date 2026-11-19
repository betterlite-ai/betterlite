use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const ACCESS_TTL_VARIABLE: &str = "DEFAULT_ACCESS_GROUP_CACHE_TTL";
const DEFAULT_ACCESS_TTL_SECONDS: i64 = 600;

// every value is read from the environment, so this section owns no YAML key
#[derive(Debug, Default, Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub access_group_ttl: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            access_group_ttl: seconds(DEFAULT_ACCESS_TTL_SECONDS),
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

// LiteLLM parses this at import: an empty or non-integer value stops startup, and a zero or
// negative value is used as given
fn access_group_ttl<L>(lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(ACCESS_TTL_VARIABLE) else {
        return Ok(seconds(DEFAULT_ACCESS_TTL_SECONDS));
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
    Ok(seconds(value))
}

fn seconds(value: i64) -> f64 {
    Decimal::from(value).to_f64().unwrap_or_default()
}
