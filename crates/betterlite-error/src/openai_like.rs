use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ExceptionCheckers, ProviderErrorFacts};
use crate::rate_limit::RateLimitCategory;

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

// the error_str predicates LiteLLM checks before ever looking at status_code
fn classify_by_predicate(error_str: &str) -> Option<(Family, &'static str)> {
    if error_str.contains("authorization denied for") {
        return Some((
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        ));
    }
    if ExceptionCheckers::context_window(error_str) {
        return Some((
            Family::ContextWindowExceeded,
            "the request exceeded the model's context window",
        ));
    }
    if error_str.contains("token_quota_reached") {
        return Some((Family::RateLimit, "the provider's token quota was reached"));
    }
    if error_str.contains("The server received an invalid response from an upstream server.") {
        return Some((
            Family::InternalServer,
            "the upstream server returned an invalid response",
        ));
    }
    if error_str.contains("model_no_support_for_function") {
        return Some((
            Family::BadRequest,
            "the model does not support this request",
        ));
    }
    None
}

// the status_code fallback LiteLLM only reaches once every error_str predicate above has missed
const fn classify_by_status(status_code: u16) -> Option<(Family, &'static str)> {
    match status_code {
        500 => Some((
            Family::InternalServer,
            "the provider reported an internal server error",
        )),
        401 => Some((
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        )),
        403 => Some((
            Family::PermissionDenied,
            "the provider denied permission for the request",
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

#[must_use]
pub fn map_openai_like_exception(
    error: &ProviderErrorFacts,
    error_str: &str,
    custom_llm_provider: &str,
    model: Option<String>,
) -> Option<ErrorClass> {
    // mirrors _map_openai_like_exception's elif chain: the error_str predicates run first and win over status_code
    let (family, reason) =
        classify_by_predicate(error_str).or_else(|| classify_by_status(error.status_code?))?;
    let context = ErrorContext {
        message: format!("{custom_llm_provider}: {reason}"),
        model,
        provider: Some(custom_llm_provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    Some(match family {
        Family::Authentication => ErrorClass::Authentication(Box::new(context)),
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(context)),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        Family::BadRequest => ErrorClass::BadRequest(Box::new(context)),
        Family::PermissionDenied => ErrorClass::PermissionDenied(Box::new(context)),
        Family::NotFound => ErrorClass::NotFound(Box::new(context)),
        Family::Timeout(original_status) => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(original_status),
        })),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
    })
}
