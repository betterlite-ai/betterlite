use std::collections::BTreeMap;

use crate::error::details::{
    ApiDetails, RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ProviderErrorFacts, RequestFacts, ResponseFacts};
use crate::rate_limit::RateLimitCategory;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusFamily {
    Authentication,
    PermissionDenied,
    NotFound,
    Timeout,
    RateLimit,
    InternalServer,
    BadGateway,
    ServiceUnavailable,
    BadRequest,
    Api,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatusMapping {
    pub family: StatusFamily,
    pub status_code: u16,
    pub message: String,
    pub response_present: bool,
}

#[must_use]
pub fn response_or_stub(error: &ProviderErrorFacts, status_code: u16) -> ResponseFacts {
    if let Some(response) = &error.response {
        return response.clone();
    }
    ResponseFacts {
        status_code,
        headers: BTreeMap::new(),
        request: Some(RequestFacts {
            method: "POST".to_owned(),
            url: "https://docs.litellm.ai/docs".to_owned(),
        }),
    }
}

#[must_use]
pub fn map_exception_by_status(
    error: &ProviderErrorFacts,
    error_str: &str,
    exception_provider: &str,
) -> Option<StatusMapping> {
    let status_code = error.status_code?;
    if status_code < 400 || error.status_code_is_synthesized {
        return None;
    }
    let family = match status_code {
        401 => StatusFamily::Authentication,
        403 => StatusFamily::PermissionDenied,
        404 => StatusFamily::NotFound,
        408 | 504 => StatusFamily::Timeout,
        429 => StatusFamily::RateLimit,
        500 => StatusFamily::InternalServer,
        502 => StatusFamily::BadGateway,
        503 => StatusFamily::ServiceUnavailable,
        400..=499 => StatusFamily::BadRequest,
        _ => StatusFamily::Api,
    };
    Some(StatusMapping {
        family,
        status_code,
        message: format!("{exception_provider} - {error_str}"),
        response_present: error.response.is_some(),
    })
}

#[must_use]
pub fn status_error_class(
    error: &ProviderErrorFacts,
    error_str: &str,
    exception_provider: &str,
    model: Option<String>,
    provider: Option<String>,
) -> Option<ErrorClass> {
    let mapping = map_exception_by_status(error, error_str, exception_provider)?;
    let response_has_request = error
        .response
        .as_ref()
        .is_some_and(|response| response.request.is_some());
    let context = ErrorContext {
        message: mapping.message.clone(),
        model,
        provider,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    Some(build_error_class(&mapping, context, response_has_request))
}

// BadRequestError drops a response without _request and falls back to its class default; Authentication, PermissionDenied and NotFound only ever reach this status_code anyway
fn build_error_class(
    mapping: &StatusMapping,
    context: ErrorContext,
    response_has_request: bool,
) -> ErrorClass {
    match mapping.family {
        StatusFamily::Authentication => ErrorClass::Authentication(Box::new(
            ResponseDetails::with_status(context, Some(mapping.status_code)),
        )),
        StatusFamily::PermissionDenied => ErrorClass::PermissionDenied(Box::new(
            ResponseDetails::with_status(context, Some(mapping.status_code)),
        )),
        StatusFamily::NotFound => ErrorClass::NotFound(Box::new(ResponseDetails::with_status(
            context,
            Some(mapping.status_code),
        ))),
        StatusFamily::Timeout => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(mapping.status_code),
        })),
        StatusFamily::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        StatusFamily::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        StatusFamily::BadGateway => ErrorClass::BadGateway(Box::new(context)),
        StatusFamily::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
        StatusFamily::BadRequest => ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
            context,
            response_has_request.then_some(mapping.status_code),
        ))),
        StatusFamily::Api => ErrorClass::Api(Box::new(ApiDetails {
            context,
            status: mapping.status_code,
        })),
    }
}
