use std::collections::BTreeMap;

use crate::error::details::{ApiDetails, RateLimitDetails, TimeoutDetails, TimeoutStatus};
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
    if let Some(class) = detail_classification(input) {
        return Some(class);
    }
    status_classification(input)
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

// "detail" in error_str always raises, independent of status_code, and runs before the status chain below
fn detail_classification(input: &NlpCloudExceptionInput) -> Option<ErrorClass> {
    if !input.error_str.contains("detail") {
        return None;
    }
    if input
        .error_str
        .contains("Input text length should not exceed")
    {
        return Some(ErrorClass::ContextWindowExceeded(Box::new(context(input))));
    }
    if input.error_str.contains("value is not a valid") {
        return Some(ErrorClass::BadRequest(Box::new(context(input))));
    }
    Some(ErrorClass::Api(Box::new(ApiDetails {
        context: context(input),
        status: 500,
    })))
}

// https://docs.nlpcloud.com/?shell#errors; every status reaching this chain raises something, even the trailing else
fn status_classification(input: &NlpCloudExceptionInput) -> Option<ErrorClass> {
    let status = input.status_code?;
    let ctx = context(input);
    Some(match status {
        400 | 406 | 413 | 422 => ErrorClass::BadRequest(Box::new(ctx)),
        401 | 403 => ErrorClass::Authentication(Box::new(ctx)),
        // Timeout.__init__ defaults exception_status_code to 408 when the caller never passes it
        522 | 524 => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context: ctx,
            original_status: TimeoutStatus(408),
        })),
        429 | 402 => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context: ctx,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        500 | 503 => ErrorClass::Api(Box::new(ApiDetails {
            context: ctx,
            status,
        })),
        // ServiceUnavailableError.__init__ hardcodes status_code=503 regardless of the original status
        504 | 520 => ErrorClass::ServiceUnavailable(Box::new(ctx)),
        other => ErrorClass::Api(Box::new(ApiDetails {
            context: ctx,
            status: other,
        })),
    })
}
