use std::collections::BTreeMap;

use crate::error::details::{
    ApiDetails, RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use crate::error::{ErrorClass, ErrorContext};
use crate::rate_limit::RateLimitCategory;

mod tests;

// mirrors the _map_nlp_cloud_exception kwargs its branch predicates read
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NlpCloudExceptionInput {
    pub model: String,
    pub error_str: String,
    pub status_code: Option<u16>,
    pub message: Option<String>,
}

#[must_use]
pub fn map_nlp_cloud_exception(input: &NlpCloudExceptionInput) -> Option<ErrorClass> {
    let family = classify(input)?;
    let context = context(input);
    Some(build(family, context, input.status_code))
}

fn context(input: &NlpCloudExceptionInput) -> ErrorContext {
    let text = input
        .message
        .clone()
        .unwrap_or_else(|| input.error_str.clone());
    ErrorContext {
        message: format!("nlp_cloud: {text}"),
        model: Some(input.model.clone()),
        provider: Some("nlp_cloud".to_owned()),
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
    Authentication,
    // Timeout.__init__ defaults exception_status_code to 408 when the caller never passes it
    Timeout,
    RateLimit,
    // ServiceUnavailableError.__init__ hardcodes status_code=503 regardless of the original status
    ServiceUnavailable,
    Api(u16),
}

type TextPredicate = fn(&str) -> bool;

const DETAIL_CANDIDATES: [(TextPredicate, Family); 2] = [
    (
        |error_str| error_str.contains("Input text length should not exceed"),
        Family::ContextWindowExceeded,
    ),
    (
        |error_str| error_str.contains("value is not a valid"),
        Family::BadRequest,
    ),
];

// "detail" in error_str always raises, independent of status_code, ahead of the status chain below (https://docs.nlpcloud.com/?shell#errors)
fn classify(input: &NlpCloudExceptionInput) -> Option<Family> {
    if input.error_str.contains("detail") {
        let found = DETAIL_CANDIDATES
            .into_iter()
            .find(|(matches, _)| matches(&input.error_str));
        return Some(found.map_or(Family::Api(500), |(_, family)| family));
    }
    Some(match input.status_code? {
        400 | 406 | 413 | 422 => Family::BadRequest,
        401 | 403 => Family::Authentication,
        522 | 524 => Family::Timeout,
        429 | 402 => Family::RateLimit,
        500 | 503 => Family::Api(input.status_code?),
        504 | 520 => Family::ServiceUnavailable,
        other => Family::Api(other),
    })
}

// every detail branch, and every status branch that carries a class, passes response=getattr(original_exception, "response", None)
fn build(family: Family, context: ErrorContext, status_code: Option<u16>) -> ErrorClass {
    match family {
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, status_code),
        )),
        Family::BadRequest => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Family::Authentication => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, status_code)))
        }
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
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
        Family::Api(status) => ErrorClass::Api(Box::new(ApiDetails { context, status })),
    }
}
