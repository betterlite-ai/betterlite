use std::collections::BTreeMap;

use crate::error::details::{
    ApiDetails, RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::ProviderErrorFacts;
use crate::rate_limit::RateLimitCategory;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    ContextWindowExceeded,
    BadRequest,
    RateLimit,
    Authentication,
    Timeout,
    ServiceUnavailable,
    Api(u16),
}

// the three error_str predicates run first, unconditionally; hasattr(status_code) is a separate `if`, not their elif
fn classify(facts: &ProviderErrorFacts, error_str: &str) -> Option<(Family, &'static str)> {
    if error_str.contains("length limit exceeded") {
        return Some((
            Family::ContextWindowExceeded,
            "the request exceeded the model's context window",
        ));
    }
    if error_str.contains("A valid user token is required") {
        return Some((Family::BadRequest, "a valid user token is required"));
    }
    if error_str.contains("Rate limit reached") {
        return Some((Family::RateLimit, "the provider rate limited the request"));
    }
    let status = facts.status_code?;
    Some(match status {
        401 => (
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        ),
        400 => (Family::BadRequest, "the provider rejected the request"),
        408 => (Family::Timeout, "the request to the provider timed out"),
        429 => (Family::RateLimit, "the provider rate limited the request"),
        503 => (
            Family::ServiceUnavailable,
            "the provider is temporarily unavailable",
        ),
        other => (
            Family::Api(other),
            "the provider returned an unmapped status",
        ),
    })
}

fn huggingface_status(facts: &ProviderErrorFacts) -> Option<u16> {
    let response_has_request = facts
        .response
        .as_ref()
        .is_some_and(|response| response.request.is_some());
    response_has_request.then_some(()).and(facts.status_code)
}

fn huggingface_context(reason: &str, model: Option<String>) -> ErrorContext {
    ErrorContext {
        message: reason.to_owned(),
        model,
        provider: Some("huggingface".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn build_huggingface_class(
    family: Family,
    context: ErrorContext,
    status_code: Option<u16>,
) -> ErrorClass {
    match family {
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, status_code),
        )),
        Family::BadRequest => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::Authentication => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Family::Timeout => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408),
        })),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
        Family::Api(status) => ErrorClass::Api(Box::new(ApiDetails { context, status })),
    }
}

#[must_use]
pub fn map_huggingface_exception(
    facts: &ProviderErrorFacts,
    error_str: &str,
    model: Option<String>,
) -> Option<ErrorClass> {
    let (family, reason) = classify(facts, error_str)?;
    let status_code = huggingface_status(facts);
    let context = huggingface_context(reason, model);
    Some(build_huggingface_class(family, context, status_code))
}
