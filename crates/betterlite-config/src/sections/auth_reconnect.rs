use std::str::FromStr;
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

const DEFAULT_COOLDOWN_SECONDS: i64 = 15;
const DEFAULT_ESCALATION_THRESHOLD: i64 = 3;
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
        let cooldown = setting(
            lookup,
            COOLDOWN_VARIABLE,
            DEFAULT_COOLDOWN_SECONDS,
            |path, source| ConfigError::InvalidInteger { path, source },
        )?;
        let escalation = setting(
            lookup,
            ESCALATION_VARIABLE,
            DEFAULT_ESCALATION_THRESHOLD,
            |path, source| ConfigError::InvalidInteger { path, source },
        )?;
        let watchdog = setting(
            lookup,
            WATCHDOG_VARIABLE,
            DEFAULT_WATCHDOG_SECONDS,
            |path, source| ConfigError::InvalidFloat { path, source },
        )?;
        let auth_timeout = setting(
            lookup,
            AUTH_TIMEOUT_VARIABLE,
            DEFAULT_AUTH_TIMEOUT_SECONDS,
            |path, source| ConfigError::InvalidFloat { path, source },
        )?;
        let auth_lock = setting(
            lookup,
            AUTH_LOCK_VARIABLE,
            DEFAULT_AUTH_LOCK_SECONDS,
            |path, source| ConfigError::InvalidFloat { path, source },
        )?;
        Ok(ReconnectSettings {
            cooldown: Duration::from_secs(u64::try_from(cooldown).unwrap_or(1).max(1)),
            escalation_threshold: u64::try_from(escalation).unwrap_or(1).max(1),
            watchdog_timeout: lower_bounded(watchdog, 1.0),
            auth_timeout: lower_bounded(auth_timeout, 0.5),
            auth_lock_timeout: lower_bounded(auth_lock, 0.0),
        })
    }
}

fn setting<T, L, F>(lookup: &L, name: &str, default: T, invalid: F) -> Result<T, ConfigError>
where
    L: EnvironmentLookup,
    T: FromStr,
    F: FnOnce(String, T::Err) -> ConfigError,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(number_error(name));
    }
    trimmed
        .parse::<T>()
        .map_err(|source| invalid(name.to_owned(), source))
}

// Python's max(lower, value): NaN and negative infinity select the lower bound, positive infinity stays
fn lower_bounded(value: f64, minimum: f64) -> Duration {
    let selected = value.max(minimum);
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
