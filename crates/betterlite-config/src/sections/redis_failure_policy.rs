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
        let mut settings = Settings::default();
        if let Some(text) = lookup.lookup(ENABLED) {
            settings.enabled = betterlite_env::flag(ENABLED, &text)?.unwrap_or(settings.enabled);
        }
        if let Some(text) = lookup.lookup(FAILURE_THRESHOLD) {
            settings.failure_threshold = betterlite_env::number::<u32>(FAILURE_THRESHOLD, &text)?
                .unwrap_or(settings.failure_threshold);
        }
        if let Some(text) = lookup.lookup(RECOVERY_TIMEOUT) {
            let seconds = betterlite_env::number::<u64>(RECOVERY_TIMEOUT, &text)?
                .unwrap_or(settings.recovery_timeout.as_secs());
            settings.recovery_timeout = Duration::from_secs(seconds);
        }
        if let Some(text) = lookup.lookup(TIMEOUT_MIN_DURATION) {
            settings.timeout_min_duration =
                fractional_seconds(TIMEOUT_MIN_DURATION, &text, settings.timeout_min_duration)?;
        }
        if let Some(text) = lookup.lookup(TIMEOUT_LOG_INTERVAL) {
            settings.timeout_log_interval =
                fractional_seconds(TIMEOUT_LOG_INTERVAL, &text, settings.timeout_log_interval)?;
        }
        Ok(settings)
    }
}

fn fractional_seconds(name: &str, text: &str, default: Duration) -> Result<Duration, ConfigError> {
    let Some(value) = betterlite_env::number::<f64>(name, text)? else {
        return Ok(default);
    };
    Duration::try_from_secs_f64(value).map_err(|_source| ConfigError::Environment {
        source: betterlite_env::EnvError::Number {
            name: name.to_owned(),
        },
    })
}
