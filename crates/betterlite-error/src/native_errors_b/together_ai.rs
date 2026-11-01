use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::rate_limit::RateLimitCategory;

mod tests;

// mirrors the _map_together_ai_exception kwargs its branch predicates read
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TogetherAiExceptionInput {
    pub model: String,
    pub error_str: String,
    pub status_code: Option<u16>,
    pub message: Option<String>,
}

#[must_use]
pub fn map_together_ai_exception(input: &TogetherAiExceptionInput) -> Option<ErrorClass> {
    if let Some(class) = predicate_classification(input) {
        return Some(class);
    }
    status_classification(input)
}

fn context(input: &TogetherAiExceptionInput) -> ErrorContext {
    let text = input
        .message
        .clone()
        .unwrap_or_else(|| input.error_str.clone());
    ErrorContext {
        message: format!("together_ai: {text}"),
        model: Some(input.model.clone()),
        provider: Some("together_ai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

// the only two fields any branch below ever reads off json.loads(error_str)
#[derive(serde::Deserialize)]
struct TogetherAiErrorBody {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_type: Option<String>,
}

// json.loads(error_str)["error"], falling back to error_str when it is not a JSON object with a string "error" field
fn error_field(error_str: &str) -> Option<String> {
    match serde_json::from_str::<TogetherAiErrorBody>(error_str) {
        Ok(body) => body.error,
        Err(_) => Some(error_str.to_owned()),
    }
}

fn error_type_is_validation(error_str: &str) -> bool {
    let Ok(body) = serde_json::from_str::<TogetherAiErrorBody>(error_str) else {
        return false;
    };
    body.error_type.as_deref() == Some("validation")
}

// the error_str predicates LiteLLM checks before ever looking at status_code
fn predicate_classification(input: &TogetherAiExceptionInput) -> Option<ErrorClass> {
    let field = error_field(&input.error_str);
    if field
        .as_deref()
        .is_some_and(|err| err.contains("`inputs` tokens + `max_new_tokens` must be <="))
    {
        return Some(ErrorClass::ContextWindowExceeded(Box::new(context(input))));
    }
    if field
        .as_deref()
        .is_some_and(|err| err.contains("invalid private key"))
    {
        return Some(ErrorClass::Authentication(Box::new(context(input))));
    }
    if field
        .as_deref()
        .is_some_and(|err| err.contains("INVALID_ARGUMENT"))
    {
        return Some(ErrorClass::BadRequest(Box::new(context(input))));
    }
    if input.error_str.contains("A timeout occurred") {
        return Some(ErrorClass::Timeout(Box::new(TimeoutDetails {
            context: context(input),
            original_status: TimeoutStatus(408),
        })));
    }
    let is_validation = field
        .as_deref()
        .is_some_and(|err| err.contains("API key doesn't match expected format."))
        || error_type_is_validation(&input.error_str);
    if is_validation {
        return Some(ErrorClass::BadRequest(Box::new(context(input))));
    }
    None
}

// a status present but unmatched, and a missing status_code attribute, both crash with AttributeError in LiteLLM (see docs/deviations/B242.yaml); betterlite falls through to None for both
fn status_classification(input: &TogetherAiExceptionInput) -> Option<ErrorClass> {
    let status = input.status_code?;
    let ctx = context(input);
    match status {
        // Timeout() never receives exception_status_code in either branch, so both default to 408
        408 | 524 => Some(ErrorClass::Timeout(Box::new(TimeoutDetails {
            context: ctx,
            original_status: TimeoutStatus(408),
        }))),
        422 => Some(ErrorClass::BadRequest(Box::new(ctx))),
        429 => Some(ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context: ctx,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        }))),
        _ => None,
    }
}
