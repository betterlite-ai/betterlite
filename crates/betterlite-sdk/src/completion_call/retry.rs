use std::time::Duration;

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::error::CompletionCallError;
use super::ports::Sleeper;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::{CallError, field_error, plain_error};
use crate::call_wrapper::state::LoggingHandle;
use crate::call_wrapper::target::CallTarget;

// completion_with_retries pops num_retries with this default
pub const DEFAULT_NUM_RETRIES: i64 = 3;
// wait_exponential(multiplier=1, max=10)
pub const MAX_BACKOFF_SECONDS: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryStrategy {
    Constant,
    ExponentialBackoff,
}

impl RetryStrategy {
    // LiteLLM names exponential_backoff_retry and takes its else branch for every other string
    pub fn parse(value: Option<&Json>) -> Result<Self, CompletionCallError> {
        match value {
            None | Some(Json::Null) => Ok(Self::Constant),
            Some(Json::Text(text)) if text == "exponential_backoff_retry" => {
                Ok(Self::ExponentialBackoff)
            }
            Some(Json::Text(_)) => Ok(Self::Constant),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                Err(CompletionCallError::Field {
                    field: "retry_strategy",
                })
            }
        }
    }

    // tenacity's wait_exponential: 2^(attempt-1) seconds, never more than ten
    #[must_use]
    pub fn wait(self, attempt: u32) -> Duration {
        match self {
            Self::Constant => Duration::ZERO,
            Self::ExponentialBackoff => {
                let exponent = attempt.saturating_sub(1).min(63);
                let seconds = 1_u64
                    .checked_shl(exponent)
                    .unwrap_or(u64::MAX)
                    .min(MAX_BACKOFF_SECONDS);
                Duration::from_secs(seconds)
            }
        }
    }
}

// The retry entry: num_retries counts attempts, the inner call gets zero retries, the last error is raised unchanged
pub async fn with_retries(
    target: &dyn CallTarget,
    mut request: RequestMap,
    logging: Option<&LoggingHandle>,
    sleeper: &dyn Sleeper,
) -> Result<CallOutcome, CallError> {
    let count = retry_count(&request)?;
    let strategy = RetryStrategy::parse(request.shift_remove("retry_strategy").as_ref())
        .map_err(|error| CallError::from(plain_error(&error.to_string())))?;
    request.shift_remove("original_function");
    request.insert(
        String::from("num_retries"),
        Json::Number(Number::from(0_i64)),
    );
    request.insert(
        String::from("max_retries"),
        Json::Number(Number::from(0_i64)),
    );
    let attempts = u32::try_from(count.max(1)).unwrap_or(u32::MAX);
    for attempt in 1..=attempts {
        match target.call(request.clone(), logging).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) if attempt == attempts => return Err(error),
            Err(_error) => sleeper.sleep(strategy.wait(attempt)).await,
        }
    }
    Err(CallError::from(plain_error(
        "the retry entry ran no attempt",
    )))
}

fn retry_count(request: &RequestMap) -> Result<i64, CallError> {
    match request.get("num_retries") {
        None | Some(Json::Null) => Ok(DEFAULT_NUM_RETRIES),
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or_else(|| field_error("num_retries", "a whole number")),
        Some(Json::Text(text)) => text
            .parse::<i64>()
            .map_err(|_error| field_error("num_retries", "a whole number")),
        Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            Err(field_error("num_retries", "a whole number"))
        }
    }
}
