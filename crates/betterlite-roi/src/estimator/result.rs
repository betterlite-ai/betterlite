use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use super::extraction::first_json_object;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResultError {
    #[error("Hours must be finite and nonnegative.")]
    Hours,
    #[error("Reasoning must not be empty.")]
    Reasoning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ROIEstimatorResult {
    #[serde(deserialize_with = "deserialize_hours")]
    pub hours: f64,
    #[serde(deserialize_with = "deserialize_reasoning")]
    pub reasoning: String,
}

fn deserialize_hours<'de, DeserializerType>(
    deserializer: DeserializerType,
) -> Result<f64, DeserializerType::Error>
where
    DeserializerType: Deserializer<'de>,
{
    let hours = f64::deserialize(deserializer)?;
    validate_hours(hours).map_err(serde::de::Error::custom)
}

fn deserialize_reasoning<'de, DeserializerType>(
    deserializer: DeserializerType,
) -> Result<String, DeserializerType::Error>
where
    DeserializerType: Deserializer<'de>,
{
    let reasoning = String::deserialize(deserializer)?;
    validate_reasoning(reasoning).map_err(serde::de::Error::custom)
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

pub fn parse_estimator_result(content: &str) -> Result<ROIEstimatorResult, serde_json::Error> {
    serde_json::from_str(first_json_object(content))
}
