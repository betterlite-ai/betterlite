use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, ResponseDetails};
use crate::error::{ErrorClass, ErrorContext};
use crate::rate_limit::RateLimitCategory;

mod tests;

// mirrors the _map_aleph_alpha_exception kwargs its branch predicates read
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AlephAlphaExceptionInput {
    pub model: String,
    pub error_str: String,
    pub status_code: Option<u16>,
    pub message: Option<String>,
}

// An unmatched status or missing status becomes the outer API connection error.
#[must_use]
pub fn map_aleph_alpha_exception(input: &AlephAlphaExceptionInput) -> Option<ErrorClass> {
    let family = classify(input).unwrap_or(Family::ApiConnection);
    let context = context(input);
    if family == Family::ApiConnection {
        let context = ErrorContext {
            message: context.message,
            model: Some(String::new()),
            provider: Some(String::new()),
            max_retries: context.max_retries,
            num_retries: context.num_retries,
            body_type: None,
            body_param: None,
            body_code: None,
        };
        return Some(ErrorClass::ApiConnection(Box::new(context)));
    }
    Some(build(family, context, input.status_code))
}

fn context(input: &AlephAlphaExceptionInput) -> ErrorContext {
    let text = input
        .message
        .clone()
        .unwrap_or_else(|| input.error_str.clone());
    ErrorContext {
        message: format!("aleph_alpha: {text}"),
        model: Some(input.model.clone()),
        provider: Some("aleph_alpha".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    ContextWindowExceeded,
    BadRequest,
    // the 401 branch raises without a response kwarg at all, so it keeps the class default
    Authentication,
    RateLimit,
    ServiceUnavailable,
    ApiConnection,
}

type TextPredicate = fn(&str) -> bool;

const TEXT_CANDIDATES: [(TextPredicate, Family); 2] = [
    (
        |error_str| error_str.contains("This is longer than the model's maximum context length"),
        Family::ContextWindowExceeded,
    ),
    (
        |error_str| error_str.contains("InvalidToken") || error_str.contains("No token provided"),
        Family::BadRequest,
    ),
];

// both text branches pass response=getattr(original_exception, "response", None), so both answer input.status_code directly
fn classify(input: &AlephAlphaExceptionInput) -> Option<Family> {
    if let Some((_, family)) = TEXT_CANDIDATES
        .into_iter()
        .find(|(matches, _)| matches(&input.error_str))
    {
        return Some(family);
    }
    match input.status_code? {
        401 => Some(Family::Authentication),
        400 => Some(Family::BadRequest),
        429 => Some(Family::RateLimit),
        500 => Some(Family::ServiceUnavailable),
        _ => None,
    }
}

fn build(family: Family, context: ErrorContext, status_code: Option<u16>) -> ErrorClass {
    match family {
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, status_code),
        )),
        Family::BadRequest => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Family::Authentication => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, None)))
        }
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
        Family::ApiConnection => ErrorClass::ApiConnection(Box::new(context)),
    }
}
