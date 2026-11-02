use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ExceptionCheckers, ProviderErrorFacts};
use crate::rate_limit::RateLimitCategory;
use crate::status::response_or_stub;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Authentication,
    ContextWindowExceeded,
    RateLimit,
    InternalServer,
    BadRequest,
    PermissionDenied,
    NotFound,
    // the source status that becomes TimeoutStatus; 408 has no status_code at all in the string-predicate branches
    Timeout(u16),
    ServiceUnavailable,
}

type Predicate = fn(&str) -> bool;

// the error_str predicates LiteLLM checks before ever looking at status_code, in order; the last field marks the two branches that answer with the response's status once it carries a request
const PREDICATE_CANDIDATES: &[(Predicate, Family, &str, bool)] = &[
    (
        |error_str| error_str.contains("authorization denied for"),
        Family::Authentication,
        "the provider rejected the request as unauthorized",
        true,
    ),
    (
        ExceptionCheckers::context_window,
        Family::ContextWindowExceeded,
        "the request exceeded the model's context window",
        true,
    ),
    (
        |error_str| error_str.contains("token_quota_reached"),
        Family::RateLimit,
        "the provider's token quota was reached",
        false,
    ),
    (
        |error_str| {
            error_str.contains("The server received an invalid response from an upstream server.")
        },
        Family::InternalServer,
        "the upstream server returned an invalid response",
        false,
    ),
    (
        |error_str| error_str.contains("model_no_support_for_function"),
        Family::BadRequest,
        "the model does not support this request",
        false,
    ),
];

fn classify_by_predicate(
    facts: &ProviderErrorFacts,
    error_str: &str,
) -> Option<(Family, &'static str, Option<u16>)> {
    let (_, family, reason, carries_status) = PREDICATE_CANDIDATES
        .iter()
        .find(|(predicate, ..)| predicate(error_str))?;
    let response_status = carries_status.then(|| {
        let response_has_request = facts
            .response
            .as_ref()
            .is_some_and(|response| response.request.is_some());
        response_has_request.then_some(()).and(facts.status_code)
    });
    Some((*family, reason, response_status.flatten()))
}

const fn plain_status_family(status_code: u16) -> Option<(Family, &'static str)> {
    match status_code {
        500 => Some((
            Family::InternalServer,
            "the provider reported an internal server error",
        )),
        401 => Some((
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        )),
        400 | 422 | 424 => Some((Family::BadRequest, "the provider rejected the request body")),
        404 => Some((
            Family::NotFound,
            "the provider could not find the requested resource",
        )),
        408 => Some((
            Family::Timeout(408),
            "the request to the provider timed out",
        )),
        429 => Some((Family::RateLimit, "the provider rate limited the request")),
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

// the status_code fallback LiteLLM only reaches once every error_str predicate above has missed; only the 403 branch ever passes a response (via _response_or_stub), so it is the only one that answers a real status
fn classify_by_status(
    facts: &ProviderErrorFacts,
    status_code: u16,
) -> Option<(Family, &'static str, Option<u16>)> {
    if status_code == 403 {
        return Some((
            Family::PermissionDenied,
            "the provider denied permission for the request",
            Some(response_or_stub(facts, 403).status_code),
        ));
    }
    let (family, reason) = plain_status_family(status_code)?;
    Some((family, reason, None))
}

fn openai_like_context(
    custom_llm_provider: &str,
    reason: &str,
    model: Option<String>,
) -> ErrorContext {
    ErrorContext {
        message: format!("{custom_llm_provider}: {reason}"),
        model,
        provider: Some(custom_llm_provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn build_openai_like_class(
    family: Family,
    context: ErrorContext,
    status: Option<u16>,
) -> ErrorClass {
    match family {
        Family::Authentication => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, status)))
        }
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, status),
        )),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        Family::BadRequest => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, status)))
        }
        Family::PermissionDenied => {
            ErrorClass::PermissionDenied(Box::new(ResponseDetails::with_status(context, status)))
        }
        Family::NotFound => {
            ErrorClass::NotFound(Box::new(ResponseDetails::with_status(context, status)))
        }
        Family::Timeout(original_status) => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(original_status),
        })),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
    }
}

#[must_use]
pub fn map_openai_like_exception(
    error: &ProviderErrorFacts,
    error_str: &str,
    custom_llm_provider: &str,
    model: Option<String>,
) -> Option<ErrorClass> {
    // mirrors _map_openai_like_exception's elif chain: the error_str predicates run first and win over status_code
    let (family, reason, status) = classify_by_predicate(error, error_str)
        .or_else(|| classify_by_status(error, error.status_code?))?;
    let context = openai_like_context(custom_llm_provider, reason, model);
    Some(build_openai_like_class(family, context, status))
}
