use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ErrorCode, ExceptionCheckers, ProviderErrorBody, ProviderErrorFacts};
use crate::rate_limit::RateLimitCategory;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    BadRequest,
    ContextWindowExceeded,
    InternalServer,
    Authentication,
    ContentPolicyViolation,
    RateLimit,
    PermissionDenied,
    NotFound,
    Timeout,
    ApiConnection,
    ServiceUnavailable,
}

// the five "quota or capacity" phrases _map_vertex_exception tries in this order before falling back to status_code
const QUOTA_PHRASES: [&str; 5] = [
    "429 Quota exceeded",
    "Quota exceeded for",
    "Resource exhausted",
    "IndexError: list index out of range",
    "429 Unable to submit request because the service is temporarily out of capacity.",
];

// branch-1153 and branch-1167: the project could not be resolved, or the payload itself was too large
fn classify_request_shape(error_str: &str) -> Option<(Family, &'static str)> {
    if error_str.contains("Vertex AI API has not been used in project")
        || error_str.contains("Unable to find your project")
    {
        return Some((
            Family::BadRequest,
            "the Vertex AI project could not be resolved",
        ));
    }
    if error_str.contains("400 Request payload size exceeds") {
        return Some((
            Family::ContextWindowExceeded,
            "the request payload exceeds Vertex AI's size limit",
        ));
    }
    None
}

// branch-1173, branch-1180, branch-1192 and branch-1199: what Vertex AI's response itself said was wrong
fn classify_response_shape(error_str: &str) -> Option<(Family, &'static str)> {
    if ExceptionCheckers::context_window(error_str) {
        return Some((
            Family::ContextWindowExceeded,
            "the request exceeded the model's context window",
        ));
    }
    if error_str.contains("None Unknown Error.") || error_str.contains("Content has no parts.") {
        return Some((
            Family::InternalServer,
            "Vertex AI returned an empty or unparsable response",
        ));
    }
    if error_str.contains("API key not valid.") {
        return Some((Family::Authentication, "the Vertex AI API key is not valid"));
    }
    if error_str.contains("403") {
        return Some((Family::BadRequest, "Vertex AI rejected the request"));
    }
    None
}

// branch-1213 and branch-1230: the response was blocked, or the request was throttled
fn classify_blocked_or_throttled(error_str: &str) -> Option<(Family, &'static str)> {
    if error_str.contains("The response was blocked.")
        || error_str.contains("Output blocked by content filtering policy")
    {
        return Some((
            Family::ContentPolicyViolation,
            "Vertex AI blocked the response",
        ));
    }
    if QUOTA_PHRASES
        .iter()
        .any(|phrase| error_str.contains(phrase))
    {
        return Some((
            Family::RateLimit,
            "Vertex AI's quota for this request was exceeded",
        ));
    }
    None
}

// branch-1250 and branch-1272: a 429 wrapped inside a 5xx envelope, or a bare internal server fault
fn classify_server_fault(
    error_str: &str,
    status_code: Option<u16>,
) -> Option<(Family, &'static str)> {
    if status_code.is_some_and(|status| (500..600).contains(&status))
        && body_error_code(error_str) == Some(429)
    {
        return Some((
            Family::RateLimit,
            "the gateway wrapped a 429 inside a 5xx envelope",
        ));
    }
    if error_str.contains("500 Internal Server Error")
        || error_str.contains("The model is overloaded.")
    {
        return Some((
            Family::InternalServer,
            "Vertex AI reported an internal server error",
        ));
    }
    None
}

// error.code read off a JSON error body, mirroring _get_body_error_code at exception_mapping_utils.py:183
fn body_error_code(error_str: &str) -> Option<i64> {
    let body: ProviderErrorBody = serde_json::from_str(error_str).ok()?;
    let nested = body.error.as_ref()?;
    match nested.code.as_ref()? {
        ErrorCode::Number(code) => Some(*code),
        ErrorCode::Text(text) => text.parse().ok(),
    }
}

// the status_code fallback LiteLLM only reaches once every error_str predicate above has missed
const fn classify_by_status(status_code: u16) -> Option<(Family, &'static str)> {
    match status_code {
        400 => Some((Family::BadRequest, "Vertex AI rejected the request body")),
        401 => Some((
            Family::Authentication,
            "the Vertex AI credentials were rejected",
        )),
        403 => Some((
            Family::PermissionDenied,
            "Vertex AI denied permission for the request",
        )),
        404 => Some((
            Family::NotFound,
            "Vertex AI could not find the requested resource",
        )),
        408 => Some((Family::Timeout, "the request to Vertex AI timed out")),
        429 => Some((Family::RateLimit, "Vertex AI rate limited the request")),
        500 => Some((
            Family::InternalServer,
            "Vertex AI reported an internal server error",
        )),
        502 => Some((Family::ApiConnection, "the connection to Vertex AI failed")),
        503 => Some((
            Family::ServiceUnavailable,
            "Vertex AI is temporarily unavailable",
        )),
        _ => None,
    }
}

#[must_use]
pub fn map_vertex_exception(
    error: &ProviderErrorFacts,
    error_str: &str,
    custom_llm_provider: &str,
    model: Option<String>,
) -> Option<ErrorClass> {
    // mirrors _map_vertex_exception's elif chain: the error_str predicates run first and win over status_code
    let (family, reason) = classify_request_shape(error_str)
        .or_else(|| classify_response_shape(error_str))
        .or_else(|| classify_blocked_or_throttled(error_str))
        .or_else(|| classify_server_fault(error_str, error.status_code))
        .or_else(|| classify_by_status(error.status_code?))?;
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
        Family::BadRequest => ErrorClass::BadRequest(Box::new(context)),
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(context)),
        Family::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        Family::Authentication => ErrorClass::Authentication(Box::new(context)),
        Family::ContentPolicyViolation => ErrorClass::ContentPolicyViolation(Box::new(context)),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::PermissionDenied => ErrorClass::PermissionDenied(Box::new(context)),
        Family::NotFound => ErrorClass::NotFound(Box::new(context)),
        Family::Timeout => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus::default(),
        })),
        Family::ApiConnection => ErrorClass::ApiConnection(Box::new(context)),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
    })
}
