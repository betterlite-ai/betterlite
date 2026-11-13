use std::time::Duration;

use betterlite_db::auth_reconnect::settings::ReconnectSettings;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const COOLDOWN_VARIABLE: &str = "PRISMA_RECONNECT_COOLDOWN_SECONDS";
const ESCALATION_VARIABLE: &str = "PRISMA_RECONNECT_ESCALATION_THRESHOLD";
const WATCHDOG_VARIABLE: &str = "PRISMA_WATCHDOG_RECONNECT_TIMEOUT_SECONDS";
const AUTH_TIMEOUT_VARIABLE: &str = "PRISMA_AUTH_RECONNECT_TIMEOUT_SECONDS";
const AUTH_LOCK_VARIABLE: &str = "PRISMA_AUTH_RECONNECT_LOCK_TIMEOUT_SECONDS";

const DEFAULT_COOLDOWN_SECONDS: u64 = 15;
const DEFAULT_ESCALATION_THRESHOLD: u64 = 3;
const DEFAULT_WATCHDOG_SECONDS: f64 = 30.0;
const DEFAULT_AUTH_TIMEOUT_SECONDS: f64 = 2.0;
const DEFAULT_AUTH_LOCK_SECONDS: f64 = 0.1;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // every value is read from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

pub type Settings = ReconnectSettings;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(ReconnectSettings {
            cooldown: Duration::from_secs(integer(
                lookup,
                COOLDOWN_VARIABLE,
                DEFAULT_COOLDOWN_SECONDS,
                1,
            )?),
            escalation_threshold: integer(
                lookup,
                ESCALATION_VARIABLE,
                DEFAULT_ESCALATION_THRESHOLD,
                1,
            )?,
            watchdog_timeout: seconds(lookup, WATCHDOG_VARIABLE, DEFAULT_WATCHDOG_SECONDS, 1.0)?,
            auth_timeout: seconds(
                lookup,
                AUTH_TIMEOUT_VARIABLE,
                DEFAULT_AUTH_TIMEOUT_SECONDS,
                0.5,
            )?,
            auth_lock_timeout: seconds(lookup, AUTH_LOCK_VARIABLE, DEFAULT_AUTH_LOCK_SECONDS, 0.0)?,
        })
    }
}

fn integer<L>(lookup: &L, name: &str, default: u64, minimum: u64) -> Result<u64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(number_error(name));
    }
    let value = trimmed
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: name.to_owned(),
            source,
        })?;
    Ok(u64::try_from(value).unwrap_or(minimum).max(minimum))
}

fn seconds<L>(lookup: &L, name: &str, default: f64, minimum: f64) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(lower_bounded(default, minimum));
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(number_error(name));
    }
    let value = trimmed
        .parse::<f64>()
        .map_err(|source| ConfigError::InvalidFloat {
            path: name.to_owned(),
            source,
        })?;
    Ok(lower_bounded(value, minimum))
}

// Python's max(lower, value): NaN and negative infinity select the lower bound, positive infinity stays
fn lower_bounded(value: f64, minimum: f64) -> Duration {
    let selected = if value > minimum { value } else { minimum };
    if selected.is_finite() {
        Duration::try_from_secs_f64(selected).unwrap_or(Duration::MAX)
    } else {
        Duration::MAX
    }
}

fn number_error(name: &str) -> ConfigError {
    ConfigError::Environment {
        source: betterlite_env::EnvError::Number {
            name: name.to_owned(),
        },
    }
}
