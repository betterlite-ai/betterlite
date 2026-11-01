use std::collections::BTreeMap;

use crate::error::details::{ApiDetails, RateLimitDetails, TimeoutDetails, TimeoutStatus};
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

#[must_use]
pub fn map_huggingface_exception(
    facts: &ProviderErrorFacts,
    error_str: &str,
    model: Option<String>,
) -> Option<ErrorClass> {
    let (family, reason) = classify(facts, error_str)?;
    let context = ErrorContext {
        message: reason.to_owned(),
        model,
        provider: Some("huggingface".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    Some(match family {
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(context)),
        Family::BadRequest => ErrorClass::BadRequest(Box::new(context)),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::Authentication => ErrorClass::Authentication(Box::new(context)),
        Family::Timeout => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408),
        })),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
        Family::Api(status) => ErrorClass::Api(Box::new(ApiDetails { context, status })),
    })
}
