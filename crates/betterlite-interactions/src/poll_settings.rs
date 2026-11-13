use std::collections::BTreeMap;
use std::num::ParseFloatError;
use std::time::{Duration, TryFromFloatSecsError};

use thiserror::Error;

pub const INITIAL_INTERVAL_VARIABLE: &str =
    "BACKGROUND_INTERACTION_COST_POLL_INITIAL_INTERVAL_SECONDS";
pub const MAX_INTERVAL_VARIABLE: &str = "BACKGROUND_INTERACTION_COST_POLL_MAX_INTERVAL_SECONDS";
pub const TIMEOUT_VARIABLE: &str = "BACKGROUND_INTERACTION_COST_POLL_TIMEOUT_SECONDS";
pub const ENABLED_VARIABLE: &str = "BACKGROUND_INTERACTION_COST_POLLING_ENABLED";

const DEFAULT_INITIAL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_MAX_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollSettings {
    pub initial_interval: Duration,
    pub max_interval: Duration,
    pub timeout: Duration,
    pub enabled: bool,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PollSettingsError {
    #[error("environment variable {name} must be a number, not {value:?}")]
    NotANumber {
        name: &'static str,
        value: String,
        #[source]
        source: ParseFloatError,
    },
    #[error("environment variable {name} must be a non-negative number of seconds, not {value:?}")]
    OutOfRange {
        name: &'static str,
        value: String,
        #[source]
        source: TryFromFloatSecsError,
    },
}

impl PollSettings {
    pub fn from_env_map(environment: &BTreeMap<String, String>) -> Result<Self, PollSettingsError> {
        Ok(Self {
            initial_interval: seconds(
                environment,
                INITIAL_INTERVAL_VARIABLE,
                DEFAULT_INITIAL_INTERVAL,
            )?,
            max_interval: seconds(environment, MAX_INTERVAL_VARIABLE, DEFAULT_MAX_INTERVAL)?,
            timeout: seconds(environment, TIMEOUT_VARIABLE, DEFAULT_TIMEOUT)?,
            // LiteLLM lowercases the text and compares it to "true"; no other spelling and no trimming
            enabled: environment
                .get(ENABLED_VARIABLE)
                .is_none_or(|text| text.to_lowercase() == "true"),
        })
    }
}

fn seconds(
    environment: &BTreeMap<String, String>,
    name: &'static str,
    default: Duration,
) -> Result<Duration, PollSettingsError> {
    let Some(text) = environment.get(name) else {
        return Ok(default);
    };
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|source| PollSettingsError::NotANumber {
            name,
            value: text.clone(),
            source,
        })?;
    match Duration::try_from_secs_f64(value) {
        Ok(duration) => Ok(duration),
        Err(source) if value.is_sign_negative() || value.is_nan() => {
            Err(PollSettingsError::OutOfRange {
                name,
                value: text.clone(),
                source,
            })
        }
        // LiteLLM's float() accepts +inf and values past Duration's range, then polls with no timeout or backoff cap
        Err(_) => Ok(Duration::MAX),
    }
}

mod tests;
