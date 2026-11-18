use std::num::TryFromIntError;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::grayswan::{OnFlaggedAction, ReasoningMode};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize as _;
use serde::de::value::{Error as ValueError, StrDeserializer};
use thiserror::Error;

pub const DEFAULT_API_BASE: &str = "https://api.grayswan.ai";
pub const DEFAULT_VIOLATION_THRESHOLD: f64 = 0.5;
// LiteLLM's `fail_open` default is true and `guardrail_timeout` defaults to 30.0 seconds
pub const DEFAULT_FAIL_OPEN: bool = true;
pub const DEFAULT_GUARDRAIL_TIMEOUT_SECONDS: f64 = 30.0;
// LiteLLM's on_flagged_action default is monitor; an unsupported value falls back to it
pub const DEFAULT_ON_FLAGGED_ACTION: OnFlaggedAction = OnFlaggedAction::Monitor;

// LiteLLM's GraySwanGuardrailMissingSecrets, raised when neither the argument nor the env var has a key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Gray Swan API key missing. Set `GRAYSWAN_API_KEY` or pass `api_key`.")]
pub struct GraySwanGuardrailMissingSecrets;

#[derive(Debug, Error)]
pub enum GraySwanGuardrailSetupError {
    #[error(transparent)]
    MissingSecrets(#[from] GraySwanGuardrailMissingSecrets),
    #[error("guardrail_timeout must be a non-negative number of seconds: {seconds}")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("streaming_sampling_rate must be a non-negative integer: {value}")]
    SamplingRate {
        value: i64,
        #[source]
        source: TryFromIntError,
    },
}

pub fn resolve_api_key(
    api_key: Option<&SecretString>,
    env: &dyn EnvironmentLookup,
) -> Result<SecretString, GraySwanGuardrailSetupError> {
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        return Ok(key.clone());
    }
    match env.lookup("GRAYSWAN_API_KEY") {
        Some(key) if !key.is_empty() => Ok(SecretString::from(key)),
        _ => Err(GraySwanGuardrailSetupError::MissingSecrets(
            GraySwanGuardrailMissingSecrets,
        )),
    }
}

#[must_use]
pub fn resolve_api_base(api_base: Option<&str>, env: &dyn EnvironmentLookup) -> String {
    let base = api_base
        .filter(|base| !base.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            env.lookup("GRAYSWAN_API_BASE")
                .filter(|base| !base.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    base.trim_end_matches('/').to_owned()
}

#[must_use]
pub fn resolve_threshold(value: Option<f64>, env: &dyn EnvironmentLookup) -> f64 {
    value.unwrap_or_else(|| env_threshold(env))
}

fn env_threshold(env: &dyn EnvironmentLookup) -> f64 {
    env.lookup("GRAYSWAN_VIOLATION_THRESHOLD")
        .and_then(|text| text.trim().parse::<f64>().ok())
        .unwrap_or(DEFAULT_VIOLATION_THRESHOLD)
}

#[must_use]
pub fn resolve_reasoning_mode(
    value: Option<&str>,
    env: &dyn EnvironmentLookup,
) -> Option<ReasoningMode> {
    value.and_then(parse_reasoning_mode).or_else(|| {
        env.lookup("GRAYSWAN_REASONING_MODE")
            .as_deref()
            .and_then(parse_reasoning_mode)
    })
}

fn parse_reasoning_mode(value: &str) -> Option<ReasoningMode> {
    let lowered = value.to_ascii_lowercase();
    ReasoningMode::deserialize(StrDeserializer::<ValueError>::new(&lowered)).ok()
}

#[must_use]
pub fn resolve_on_flagged_action(action: Option<&str>) -> OnFlaggedAction {
    let Some(text) = action.filter(|text| !text.is_empty()) else {
        return DEFAULT_ON_FLAGGED_ACTION;
    };
    match text.to_ascii_lowercase().as_str() {
        "block" => OnFlaggedAction::Block,
        "monitor" => OnFlaggedAction::Monitor,
        "passthrough" => OnFlaggedAction::Passthrough,
        _ => {
            tracing::warn!(
                on_flagged_action = text,
                "Gray Swan Guardrail: unsupported on_flagged_action, defaulting to monitor"
            );
            DEFAULT_ON_FLAGGED_ACTION
        }
    }
}

pub fn resolve_timeout(value: Option<f64>) -> Result<Duration, GraySwanGuardrailSetupError> {
    let seconds = value.unwrap_or(DEFAULT_GUARDRAIL_TIMEOUT_SECONDS);
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| GraySwanGuardrailSetupError::Timeout { seconds, source })
}

pub fn resolve_sampling_rate(value: i64) -> Result<u32, GraySwanGuardrailSetupError> {
    u32::try_from(value)
        .map_err(|source| GraySwanGuardrailSetupError::SamplingRate { value, source })
}
