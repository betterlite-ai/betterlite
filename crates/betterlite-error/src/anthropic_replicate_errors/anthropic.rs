use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::ExceptionCheckers;
use crate::rate_limit::RateLimitCategory;

mod tests;

// mirrors the _map_anthropic_exception kwargs read by its branch predicates
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnthropicExceptionInput {
    pub model: String,
    pub error_str: String,
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    ContextWindowExceeded,
    InternalServer,
    Authentication,
    ContentPolicyViolation,
    BadRequest,
    PermissionDenied,
    NotFound,
    Timeout(u16),
    RateLimit,
    BadGateway,
    ServiceUnavailable,
}

// the error_str predicates checked in source order, each an independent `if`, before status_code is ever read
fn classify_by_predicate(error_str: &str) -> Option<(Family, &'static str)> {
    if error_str.contains("prompt is too long")
        || error_str.contains("prompt: length")
        || ExceptionCheckers::context_window(error_str)
    {
        return Some((
            Family::ContextWindowExceeded,
            "the prompt exceeded the model's context window",
        ));
    }
    if error_str.contains("overloaded_error") || error_str.contains("Overloaded") {
        return Some((
            Family::InternalServer,
            "the provider reported it is overloaded",
        ));
    }
    if error_str.contains("Invalid API Key") {
        return Some((Family::Authentication, "the provider rejected the api key"));
    }
    if error_str.contains("content filtering policy") {
        return Some((
            Family::ContentPolicyViolation,
            "the content filtering policy blocked the request",
        ));
    }
    if error_str.contains("Client error '400 Bad Request'") {
        return Some((
            Family::BadRequest,
            "the provider reported a 400 Bad Request",
        ));
    }
    None
}

// the status_code elif chain, only reached once every error_str predicate above has missed
const fn classify_by_status(status_code: u16) -> Option<(Family, &'static str)> {
    match status_code {
        401 => Some((
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        )),
        403 => Some((
            Family::PermissionDenied,
            "the provider denied permission for the request",
        )),
        400 | 413 => Some((Family::BadRequest, "the provider rejected the request body")),
        404 => Some((
            Family::NotFound,
            "the provider could not find the requested resource",
        )),
        408 => Some((
            Family::Timeout(408),
            "the request to the provider timed out",
        )),
        429 => Some((Family::RateLimit, "the provider rate limited the request")),
        500 | 529 => Some((
            Family::InternalServer,
            "the provider reported an internal server error",
        )),
        502 => Some((
            Family::BadGateway,
            "the provider's gateway returned a bad response",
        )),
        503 => Some((
            Family::ServiceUnavailable,
            "the provider is temporarily unavailable",
        )),
        504 => Some((
            Family::Timeout(504),
            "the gateway to the provider timed out",
        )),
        _ => None,
    }
}

#[must_use]
pub fn map_anthropic_exception(input: &AnthropicExceptionInput) -> Option<ErrorClass> {
    let (family, reason) = classify_by_predicate(&input.error_str)
        .or_else(|| classify_by_status(input.status_code?))?;
    let context = ErrorContext {
        message: format!("anthropic: {reason}"),
        model: Some(input.model.clone()),
        provider: Some("anthropic".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    Some(match family {
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(context)),
        Family::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        Family::Authentication => ErrorClass::Authentication(Box::new(context)),
        Family::ContentPolicyViolation => ErrorClass::ContentPolicyViolation(Box::new(context)),
        Family::BadRequest => ErrorClass::BadRequest(Box::new(context)),
        Family::PermissionDenied => ErrorClass::PermissionDenied(Box::new(context)),
        Family::NotFound => ErrorClass::NotFound(Box::new(context)),
        Family::Timeout(original_status) => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(original_status),
        })),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::BadGateway => ErrorClass::BadGateway(Box::new(context)),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
    })
}
