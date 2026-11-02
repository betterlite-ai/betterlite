use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus};
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
    let family = classify(input)?;
    let context = context(input);
    Some(build(family, context, input.status_code))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    ContextWindowExceeded,
    Authentication,
    BadRequest,
    Timeout,
    RateLimit,
}

// the error_str predicates LiteLLM checks before ever looking at status_code
fn predicate_family(input: &TogetherAiExceptionInput) -> Option<Family> {
    let field = error_field(&input.error_str);
    if field
        .as_deref()
        .is_some_and(|err| err.contains("`inputs` tokens + `max_new_tokens` must be <="))
    {
        return Some(Family::ContextWindowExceeded);
    }
    if field
        .as_deref()
        .is_some_and(|err| err.contains("invalid private key"))
    {
        return Some(Family::Authentication);
    }
    if field
        .as_deref()
        .is_some_and(|err| err.contains("INVALID_ARGUMENT"))
    {
        return Some(Family::BadRequest);
    }
    if input.error_str.contains("A timeout occurred") {
        return Some(Family::Timeout);
    }
    let is_validation = field
        .as_deref()
        .is_some_and(|err| err.contains("API key doesn't match expected format."))
        || error_type_is_validation(&input.error_str);
    is_validation.then_some(Family::BadRequest)
}

// a status present but unmatched, and a missing status_code attribute, both crash with AttributeError in LiteLLM (see docs/deviations/B242.yaml); betterlite falls through to None for both
fn classify(input: &TogetherAiExceptionInput) -> Option<Family> {
    if let Some(family) = predicate_family(input) {
        return Some(family);
    }
    // Timeout() never receives exception_status_code in either branch, so both default to 408
    match input.status_code? {
        408 | 524 => Some(Family::Timeout),
        422 => Some(Family::BadRequest),
        429 => Some(Family::RateLimit),
        _ => None,
    }
}

fn build(family: Family, context: ErrorContext, status_code: Option<u16>) -> ErrorClass {
    match family {
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, status_code),
        )),
        Family::Authentication => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Family::BadRequest => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Family::Timeout => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408),
        })),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
    }
}
