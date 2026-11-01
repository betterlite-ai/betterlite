use std::collections::BTreeMap;

use crate::error::details::RateLimitDetails;
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

// a status present but unmatched, and a missing status_code attribute, both crash with AttributeError in LiteLLM; betterlite has nothing typed to hand back in either case, so both fall through as None
#[must_use]
pub fn map_aleph_alpha_exception(input: &AlephAlphaExceptionInput) -> Option<ErrorClass> {
    if let Some(class) = text_classification(input) {
        return Some(class);
    }
    status_classification(input)
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

type TextPredicate = fn(&str) -> bool;
type ErrorClassBuilder = fn(Box<ErrorContext>) -> ErrorClass;

const TEXT_CANDIDATES: [(TextPredicate, ErrorClassBuilder); 2] = [
    (
        |error_str| error_str.contains("This is longer than the model's maximum context length"),
        ErrorClass::ContextWindowExceeded,
    ),
    (
        |error_str| error_str.contains("InvalidToken") || error_str.contains("No token provided"),
        ErrorClass::BadRequest,
    ),
];

fn text_classification(input: &AlephAlphaExceptionInput) -> Option<ErrorClass> {
    let (_, build) = TEXT_CANDIDATES
        .into_iter()
        .find(|(matches, _)| matches(&input.error_str))?;
    Some(build(Box::new(context(input))))
}

fn status_classification(input: &AlephAlphaExceptionInput) -> Option<ErrorClass> {
    let status = input.status_code?;
    let ctx = context(input);
    match status {
        401 => Some(ErrorClass::Authentication(Box::new(ctx))),
        400 => Some(ErrorClass::BadRequest(Box::new(ctx))),
        429 => Some(ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context: ctx,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        }))),
        500 => Some(ErrorClass::ServiceUnavailable(Box::new(ctx))),
        _ => None,
    }
}
