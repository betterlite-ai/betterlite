use std::collections::BTreeMap;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_env::{EnvError, flag, number};
use thiserror::Error;

pub const ENABLE_ENV: &str = "GCS_MOCK";
pub const LATENCY_ENV: &str = "GCS_MOCK_LATENCY_MS";
pub const DEFAULT_LATENCY: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcsMockSettings {
    pub enabled: bool,
    pub latency: Duration,
}

#[derive(Debug, Error)]
pub enum GcsMockConfigError {
    #[error("the GCS mock environment is invalid")]
    Environment {
        #[source]
        source: EnvError,
    },
    #[error("GCS_MOCK_LATENCY_MS must be a non-negative number of milliseconds")]
    Latency {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("GCS_MOCK_LATENCY_MS is too large")]
    LatencyOverflow,
}

impl GcsMockSettings {
    pub fn from_environment(
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, GcsMockConfigError> {
        Ok(Self {
            enabled: should_use_gcs_mock(environment)?,
            latency: latency(environment)?,
        })
    }
}

pub fn should_use_gcs_mock(
    environment: &BTreeMap<String, String>,
) -> Result<bool, GcsMockConfigError> {
    let Some(text) = environment.get(ENABLE_ENV) else {
        return Ok(false);
    };
    let enabled =
        flag(ENABLE_ENV, text).map_err(|source| GcsMockConfigError::Environment { source })?;
    Ok(enabled.unwrap_or_default())
}

fn latency(environment: &BTreeMap<String, String>) -> Result<Duration, GcsMockConfigError> {
    let Some(text) = environment.get(LATENCY_ENV) else {
        return Ok(DEFAULT_LATENCY);
    };
    let parsed = number::<f64>(LATENCY_ENV, text)
        .map_err(|source| GcsMockConfigError::Environment { source })?;
    let Some(milliseconds) = parsed else {
        return Ok(DEFAULT_LATENCY);
    };
    // LiteLLM divides the millisecond float by 1000; scaling seconds back keeps the checked conversion
    let seconds = Duration::try_from_secs_f64(milliseconds)
        .map_err(|source| GcsMockConfigError::Latency { source })?;
    seconds
        .checked_div(1000)
        .ok_or(GcsMockConfigError::LatencyOverflow)
}
