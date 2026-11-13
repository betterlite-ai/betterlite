use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResultError {
    #[error("Hours must be finite and nonnegative.")]
    Hours,
    #[error("Reasoning must not be empty.")]
    Reasoning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawEstimatorResult")]
pub struct ROIEstimatorResult {
    pub hours: f64,
    pub reasoning: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEstimatorResult {
    hours: f64,
    reasoning: String,
}

impl TryFrom<RawEstimatorResult> for ROIEstimatorResult {
    type Error = ResultError;

    fn try_from(raw: RawEstimatorResult) -> Result<Self, ResultError> {
        Ok(Self {
            hours: validate_hours(raw.hours)?,
            reasoning: validate_reasoning(raw.reasoning)?,
        })
    }
}

pub fn validate_hours(hours: f64) -> Result<f64, ResultError> {
    if !hours.is_finite() || hours < 0.0 {
        return Err(ResultError::Hours);
    }
    Ok(hours)
}

pub fn validate_reasoning(reasoning: String) -> Result<String, ResultError> {
    if reasoning.trim().is_empty() {
        return Err(ResultError::Reasoning);
    }
    Ok(reasoning)
}
