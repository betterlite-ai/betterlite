use betterlite_models::decisions::Json;
use serde::Serialize;
use thiserror::Error;

use crate::spend_event::event::Mapping;

const PROMPT_TOKENS: &str = "prompt_tokens";
const COMPLETION_TOKENS: &str = "completion_tokens";
const TOTAL_TOKENS: &str = "total_tokens";

#[derive(Debug, Error)]
pub enum UsageError {
    #[error("the usage field {field} is not an integer that fits 64 bits")]
    Count { field: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct UsageCounts {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RestoredUsage {
    #[serde(flatten)]
    pub counts: UsageCounts,
    #[serde(flatten)]
    pub rest: Mapping,
}

// LiteLLM's Usage reads a missing or null count as 0, and any other non-integer is a validation error
fn count(fields: &Mapping, field: &'static str) -> Result<i64, UsageError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(UsageError::Count { field }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(UsageError::Count { field })
        }
    }
}

pub fn usage_counts(fields: &Mapping) -> Result<UsageCounts, UsageError> {
    Ok(UsageCounts {
        prompt_tokens: count(fields, PROMPT_TOKENS)?,
        completion_tokens: count(fields, COMPLETION_TOKENS)?,
        total_tokens: count(fields, TOTAL_TOKENS)?,
    })
}

pub fn restore_usage(mut fields: Mapping) -> Result<RestoredUsage, UsageError> {
    let counts = usage_counts(&fields)?;
    for field in [PROMPT_TOKENS, COMPLETION_TOKENS, TOTAL_TOKENS] {
        fields.shift_remove(field);
    }
    Ok(RestoredUsage {
        counts,
        rest: fields,
    })
}
