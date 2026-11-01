use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::rate_limit::RateLimitCategory;

mod tests;

// mirrors the _map_sagemaker_exception kwargs read by its branch predicates
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SagemakerExceptionInput {
    pub model: String,
    pub custom_llm_provider: String,
    pub error_str: String,
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    BadRequest,
    ContextWindowExceeded,
    Authentication,
    NotFound,
    RateLimit,
    ServiceUnavailable,
    // the source status that becomes TimeoutStatus
    Timeout(u16),
}

const fn reason(family: Family) -> &'static str {
    match family {
        Family::BadRequest => "the provider rejected the request body",
        Family::ContextWindowExceeded => "the request exceeded the model's context window",
        Family::Authentication => "the provider rejected the request as unauthenticated",
        Family::NotFound => "the provider could not find the requested resource",
        Family::RateLimit => "the provider rate limited the request",
        Family::ServiceUnavailable => "the provider is temporarily unavailable",
        Family::Timeout(_) => "the request to the provider timed out",
    }
}

// the error_str elif chain LiteLLM checks before ever looking at status_code; sagemaker always keeps the literal provider here
fn classify_by_predicate(error_str: &str) -> Option<Family> {
    if error_str.contains("Unable to locate credentials") {
        return Some(Family::BadRequest);
    }
    if error_str.contains("Input validation error: `best_of` must be > 0 and <= 2") {
        return Some(Family::BadRequest);
    }
    if error_str.contains("`inputs` tokens + `max_new_tokens` must be <=")
        || error_str.contains("instance type with more CPU capacity or memory")
    {
        return Some(Family::ContextWindowExceeded);
    }
    None
}

// the status_code fallback LiteLLM only reaches once every error_str predicate above has missed; always the custom provider
const fn classify_by_status(status_code: u16) -> Option<Family> {
    match status_code {
        500 | 503 => Some(Family::ServiceUnavailable),
        401 => Some(Family::Authentication),
        400 | 422 | 424 => Some(Family::BadRequest),
        404 => Some(Family::NotFound),
        408 => Some(Family::Timeout(408)),
        429 => Some(Family::RateLimit),
        504 => Some(Family::Timeout(504)),
        _ => None,
    }
}

#[must_use]
pub fn map_sagemaker_exception(input: &SagemakerExceptionInput) -> Option<ErrorClass> {
    if let Some(family) = classify_by_predicate(&input.error_str) {
        return Some(build(input, family, "sagemaker".to_owned()));
    }
    let family = classify_by_status(input.status_code?)?;
    Some(build(input, family, input.custom_llm_provider.clone()))
}

fn build(input: &SagemakerExceptionInput, family: Family, provider: String) -> ErrorClass {
    let context = ErrorContext {
        message: format!("{provider}: {}", reason(family)),
        model: Some(input.model.clone()),
        provider: Some(provider),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    match family {
        Family::BadRequest => ErrorClass::BadRequest(Box::new(context)),
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(context)),
        Family::Authentication => ErrorClass::Authentication(Box::new(context)),
        Family::NotFound => ErrorClass::NotFound(Box::new(context)),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
        Family::Timeout(original_status) => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(original_status),
        })),
    }
}
