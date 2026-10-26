use serde::Deserialize;
use std::num::ParseFloatError;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct StructuredTimeout {
    #[serde(default)]
    pub connect: Option<f64>,
    #[serde(default)]
    pub read: Option<f64>,
    #[serde(default)]
    pub write: Option<f64>,
    #[serde(default)]
    pub pool: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FineTuningTimeout {
    Seconds(f64),
    Structured(StructuredTimeout),
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TimeoutError {
    #[error("timeout must be finite")]
    NonFinite,
    #[error("timeout is not a valid number: {source}")]
    InvalidNumber { source: ParseFloatError },
}

#[must_use]
pub fn supports_httpx_timeout(provider: &str) -> bool {
    matches!(provider, "openai" | "azure" | "bedrock")
}

pub fn resolve_fine_tuning_timeout(
    raw: Option<&TimeoutValue>,
    provider: &str,
    structured: Option<StructuredTimeout>,
) -> Result<FineTuningTimeout, TimeoutError> {
    if let Some(timeout) = structured {
        if supports_httpx_timeout(provider) {
            validate_structured(timeout)?;
            return Ok(FineTuningTimeout::Structured(timeout));
        }
        return Ok(FineTuningTimeout::Seconds(finite_or_default(timeout.read)?));
    }
    let seconds = match raw {
        None => 600.0_f64,
        Some(TimeoutValue::Number(value)) if *value == 0.0_f64 => 600.0_f64,
        Some(TimeoutValue::Number(value)) => *value,
        Some(TimeoutValue::String(value)) if value.is_empty() => 600.0_f64,
        Some(TimeoutValue::String(value)) => parse_timeout_string(value)?,
    };
    if !seconds.is_finite() {
        return Err(TimeoutError::NonFinite);
    }
    Ok(FineTuningTimeout::Seconds(seconds))
}

pub fn request_timeout(
    timeout: Option<f64>,
    request_timeout: Option<f64>,
) -> Result<f64, TimeoutError> {
    let selected = timeout
        .filter(|value| *value != 0.0_f64)
        .or_else(|| request_timeout.filter(|value| *value != 0.0_f64))
        .unwrap_or(600.0_f64);
    if selected.is_finite() {
        Ok(selected)
    } else {
        Err(TimeoutError::NonFinite)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TimeoutValue {
    Number(f64),
    String(String),
}

fn finite_or_default(value: Option<f64>) -> Result<f64, TimeoutError> {
    let timeout = value
        .filter(|number| *number != 0.0_f64)
        .unwrap_or(600.0_f64);
    if timeout.is_finite() {
        Ok(timeout)
    } else {
        Err(TimeoutError::NonFinite)
    }
}

fn parse_timeout_string(value: &str) -> Result<f64, TimeoutError> {
    let trimmed = value.trim();
    let valid_underscores = trimmed
        .chars()
        .zip(trimmed.chars().skip(1))
        .all(|(left, right)| {
            (left != '_' || right.is_ascii_digit()) && (right != '_' || left.is_ascii_digit())
        });
    let candidate = if valid_underscores && !trimmed.starts_with('_') && !trimmed.ends_with('_') {
        trimmed.replace('_', "")
    } else {
        trimmed.to_owned()
    };
    candidate
        .parse::<f64>()
        .map_err(|source| TimeoutError::InvalidNumber { source })
}

fn validate_structured(timeout: StructuredTimeout) -> Result<(), TimeoutError> {
    if [timeout.connect, timeout.read, timeout.write, timeout.pool]
        .into_iter()
        .flatten()
        .all(f64::is_finite)
    {
        Ok(())
    } else {
        Err(TimeoutError::NonFinite)
    }
}
