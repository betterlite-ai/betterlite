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
    Authentication,
    Timeout,
    RateLimit,
    Api(u16),
}

// only runs when message is present; either sub-check can match independently, both plain `if`, not `if`/`elif`
fn classify_by_message(message: Option<&str>) -> Option<(Family, &'static str)> {
    let text = message?;
    if text.contains("Prompt has too many tokens") {
        return Some((
            Family::ContextWindowExceeded,
            "the request exceeded the model's context window",
        ));
    }
    if text.contains("Bad or missing API token.") {
        return Some((
            Family::BadRequest,
            "the provider rejected the request as unauthorized",
        ));
    }
    None
}

// a separate `if hasattr(status_code)` block; 422 is a plain `if` nested after 401/408, not their elif
const fn classify_by_status(status: u16) -> (Family, &'static str) {
    match status {
        401 => (
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        ),
        408 => (Family::Timeout, "the request to the provider timed out"),
        422 => (Family::BadRequest, "the provider rejected the request body"),
        429 => (Family::RateLimit, "the provider rate limited the request"),
        other => (
            Family::Api(other),
            "the provider returned an unmapped status",
        ),
    }
}

fn ai21_context(reason: &str, model: Option<String>) -> ErrorContext {
    ErrorContext {
        message: reason.to_owned(),
        model,
        provider: Some("ai21".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

// the three response-carrying families keep their class default unless a response with a request accompanied the original exception
fn ai21_status(facts: &ProviderErrorFacts) -> Option<u16> {
    let response_has_request = facts
        .response
        .as_ref()
        .is_some_and(|response| response.request.is_some());
    response_has_request.then_some(()).and(facts.status_code)
}

fn build_ai21_class(
    family: Family,
    context: ErrorContext,
    facts: &ProviderErrorFacts,
) -> ErrorClass {
    match family {
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, ai21_status(facts)),
        )),
        Family::BadRequest => ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
            context,
            ai21_status(facts),
        ))),
        Family::Authentication => ErrorClass::Authentication(Box::new(
            ResponseDetails::with_status(context, ai21_status(facts)),
        )),
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
        Family::Api(status) => ErrorClass::Api(Box::new(ApiDetails { context, status })),
    }
}

#[must_use]
pub fn map_ai21_exception(facts: &ProviderErrorFacts, model: Option<String>) -> Option<ErrorClass> {
    let (family, reason) = classify_by_message(facts.message.as_deref())
        .or_else(|| facts.status_code.map(classify_by_status))?;
    let context = ai21_context(reason, model);
    Some(build_ai21_class(family, context, facts))
}
