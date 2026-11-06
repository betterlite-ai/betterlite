use std::str::FromStr;
use std::time::Duration;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const ENABLED: &str = "REDIS_CIRCUIT_BREAKER_ENABLED";
const FAILURE_THRESHOLD: &str = "REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD";
const RECOVERY_TIMEOUT: &str = "REDIS_CIRCUIT_BREAKER_RECOVERY_TIMEOUT";
const TIMEOUT_MIN_DURATION: &str = "REDIS_CIRCUIT_BREAKER_TIMEOUT_MIN_DURATION";
const TIMEOUT_LOG_INTERVAL: &str = "REDIS_TIMEOUT_LOG_INTERVAL";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // an environment-only section must still parse as a YAML mapping
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub failure_threshold: u32,
    pub recovery_timeout: Duration,
    pub timeout_min_duration: Duration,
    pub timeout_log_interval: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(60),
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(5),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let defaults = Settings::default();
        Ok(Settings {
            enabled: flag(lookup, ENABLED, defaults.enabled)?,
            failure_threshold: integer(lookup, FAILURE_THRESHOLD, defaults.failure_threshold)?,
            recovery_timeout: whole_seconds(lookup, RECOVERY_TIMEOUT, defaults.recovery_timeout)?,
            timeout_min_duration: seconds(
                lookup,
                TIMEOUT_MIN_DURATION,
                defaults.timeout_min_duration,
            )?,
            timeout_log_interval: seconds(
                lookup,
                TIMEOUT_LOG_INTERVAL,
                defaults.timeout_log_interval,
            )?,
        })
    }
}

fn flag<L>(lookup: &L, name: &str, default: bool) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    Ok(betterlite_env::flag(name, &text)?.unwrap_or(default))
}

fn integer<L, T>(lookup: &L, name: &str, default: T) -> Result<T, ConfigError>
where
    L: EnvironmentLookup,
    T: FromStr,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    Ok(betterlite_env::number::<T>(name, &text)?.unwrap_or(default))
}

fn whole_seconds<L>(lookup: &L, name: &str, default: Duration) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    let Some(seconds) = betterlite_env::number::<u64>(name, &text)? else {
        return Ok(default);
    };
    Ok(Duration::from_secs(seconds))
}

fn seconds<L>(lookup: &L, name: &str, default: Duration) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    let Some(value) = betterlite_env::number::<f64>(name, &text)? else {
        return Ok(default);
    };
    Duration::try_from_secs_f64(value).map_err(|_source| ConfigError::Environment {
        source: betterlite_env::EnvError::Number {
            name: name.to_owned(),
        },
    })
}
