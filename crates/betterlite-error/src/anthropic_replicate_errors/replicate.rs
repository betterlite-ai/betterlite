use std::collections::BTreeMap;

use crate::error::details::{
    ApiDetails, RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use crate::error::{ErrorClass, ErrorContext};
use crate::rate_limit::RateLimitCategory;

mod tests;

// mirrors the _map_replicate_exception kwargs read by its branch predicates
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplicateExceptionInput {
    pub model: String,
    pub error_str: String,
    pub exception_type: String,
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Authentication,
    ContextWindowExceeded,
    BadRequest,
    RateLimit,
    UnprocessableEntity,
    Timeout(u16),
    ServiceUnavailable,
}

// the error_str/exception_type predicates checked in source order, before status_code is ever read
fn classify_by_predicate(error_str: &str, exception_type: &str) -> Option<(Family, &'static str)> {
    if error_str.contains("Incorrect authentication token") {
        return Some((
            Family::Authentication,
            "the provider rejected the authentication token",
        ));
    }
    if error_str.contains("input is too long") {
        return Some((
            Family::ContextWindowExceeded,
            "the input exceeded the model's context window",
        ));
    }
    if exception_type == "ModelError" {
        return Some((Family::BadRequest, "the model rejected the request"));
    }
    if error_str.contains("Request was throttled") {
        return Some((Family::RateLimit, "the provider throttled the request"));
    }
    None
}

// the status_code elif chain, only reached once every error_str/exception_type predicate above has missed
const fn classify_by_status(status_code: u16) -> Option<(Family, &'static str)> {
    match status_code {
        401 => Some((
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        )),
        400 | 413 => Some((Family::BadRequest, "the provider rejected the request body")),
        422 => Some((
            Family::UnprocessableEntity,
            "the provider could not process the request",
        )),
        408 => Some((
            Family::Timeout(408),
            "the request to the provider timed out",
        )),
        429 => Some((Family::RateLimit, "the provider rate limited the request")),
        500 => Some((
            Family::ServiceUnavailable,
            "the provider is temporarily unavailable",
        )),
        _ => None,
    }
}

fn replicate_context(reason: &str, model: Option<String>) -> ErrorContext {
    ErrorContext {
        message: format!("replicate: {reason}"),
        model,
        provider: Some("replicate".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

// _map_replicate_exception passes a response on every branch of these four families
fn build_replicate_class(
    family: Option<Family>,
    context: ErrorContext,
    status_code: Option<u16>,
) -> ErrorClass {
    match family {
        Some(Family::Authentication) => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Some(Family::ContextWindowExceeded) => ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, status_code),
        )),
        Some(Family::BadRequest) => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Some(Family::RateLimit) => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Some(Family::UnprocessableEntity) => ErrorClass::UnprocessableEntity(Box::new(
            ResponseDetails::with_status(context, status_code),
        )),
        Some(Family::Timeout(original_status)) => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(original_status),
        })),
        Some(Family::ServiceUnavailable) => ErrorClass::ServiceUnavailable(Box::new(context)),
        None => ErrorClass::Api(Box::new(ApiDetails {
            context,
            status: 500,
        })),
    }
}

#[must_use]
pub fn map_replicate_exception(input: &ReplicateExceptionInput) -> ErrorClass {
    let matched = classify_by_predicate(&input.error_str, &input.exception_type)
        .or_else(|| input.status_code.and_then(classify_by_status));
    let reason = matched.map_or("the provider returned an unmapped error", |(_, reason)| {
        reason
    });
    let context = replicate_context(reason, Some(input.model.clone()));
    build_replicate_class(
        matched.map(|(family, _)| family),
        context,
        input.status_code,
    )
}
