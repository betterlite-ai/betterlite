use std::fmt;

use serde::de::{Deserializer, Error as DeError, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use thiserror::Error;

const FIELDS: &[&str] = &["hours", "reasoning"];

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

struct RawEstimatorResult {
    hours: f64,
    reasoning: String,
}

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for RawEstimatorResult {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(RawEstimatorVisitor)
    }
}

struct RawEstimatorVisitor;

// ast-grep-ignore: no-custom-serde
impl<'de> Visitor<'de> for RawEstimatorVisitor {
    type Value = RawEstimatorResult;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object with hours and reasoning")
    }

    fn visit_map<A>(self, mut map: A) -> Result<RawEstimatorResult, A::Error>
    where
        A: MapAccess<'de>,
    {
        // LiteLLM's pydantic keeps the last value of a repeated key and validates only that one
        let mut hours_value: Option<Box<RawValue>> = None;
        let mut reasoning_value: Option<Box<RawValue>> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "hours" => hours_value = Some(map.next_value()?),
                "reasoning" => reasoning_value = Some(map.next_value()?),
                unknown => {
                    return Err(<A::Error as DeError>::unknown_field(unknown, FIELDS));
                }
            }
        }
        let hours = hours_value.ok_or_else(|| <A::Error as DeError>::missing_field("hours"))?;
        let reasoning =
            reasoning_value.ok_or_else(|| <A::Error as DeError>::missing_field("reasoning"))?;
        Ok(RawEstimatorResult {
            hours: serde_json::from_str(hours.get()).map_err(<A::Error as DeError>::custom)?,
            reasoning: serde_json::from_str(reasoning.get())
                .map_err(<A::Error as DeError>::custom)?,
        })
    }
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
    if !hours.is_finite() || hours < 0.0_f64 {
        return Err(ResultError::Hours);
    }
    Ok(hours)
}

pub fn validate_reasoning(reasoning: String) -> Result<String, ResultError> {
    // Python's str.strip also drops the four information separators, which char::is_whitespace keeps
    let has_text = reasoning
        .chars()
        .any(|character| !character.is_whitespace() && !matches!(character, '\u{1c}'..='\u{1f}'));
    if !has_text {
        return Err(ResultError::Reasoning);
    }
    Ok(reasoning)
}
