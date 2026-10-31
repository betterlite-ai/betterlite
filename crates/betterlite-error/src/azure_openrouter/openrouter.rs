use std::collections::BTreeMap;

use crate::error::details::{ApiDetails, RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::rate_limit::RateLimitCategory;

mod tests;

// mirrors the _map_openrouter_exception kwargs read by its predicates and raised exceptions
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenrouterExceptionInput {
    pub model: String,
    pub custom_llm_provider: String,
    pub error_str: String,
    pub exception_provider: String,
    pub status_code: Option<u16>,
    #[serde(default)]
    pub status_code_is_synthesized: bool,
}

#[must_use]
pub fn map_openrouter_exception(input: &OpenrouterExceptionInput) -> ErrorClass {
    match input.status_code {
        Some(status) if !input.status_code_is_synthesized => status_classification(input, status),
        _ => {
            let context = ErrorContext {
                message: format!(
                    "APIConnectionError: {} - {}",
                    input.exception_provider, input.error_str
                ),
                model: Some(input.model.clone()),
                provider: Some(input.custom_llm_provider.clone()),
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            };
            ErrorClass::ApiConnection(Box::new(context))
        }
    }
}

fn status_message(input: &OpenrouterExceptionInput, status: u16) -> String {
    let provider = &input.exception_provider;
    let text = &input.error_str;
    match status {
        400 => format!("{provider} - {text}"),
        401 => format!("AuthenticationError: {provider} - {text}"),
        404 => format!("NotFoundError: {provider} - {text}"),
        408 | 504 => format!("Timeout Error: {provider} - {text}"),
        422 => format!("BadRequestError: {provider} - {text}"),
        429 => format!("RateLimitError: {provider} - {text}"),
        503 => format!("ServiceUnavailableError: {provider} - {text}"),
        _ => format!("APIError: {provider} - {text}"),
    }
}

fn status_classification(input: &OpenrouterExceptionInput, status: u16) -> ErrorClass {
    let context = ErrorContext {
        message: status_message(input, status),
        model: Some(input.model.clone()),
        provider: Some(input.custom_llm_provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    match status {
        400 | 422 => ErrorClass::BadRequest(Box::new(context)),
        401 => ErrorClass::Authentication(Box::new(context)),
        404 => ErrorClass::NotFound(Box::new(context)),
        408 => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408),
        })),
        429 => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        503 => ErrorClass::ServiceUnavailable(Box::new(context)),
        504 => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(504),
        })),
        other => ErrorClass::Api(Box::new(ApiDetails {
            context,
            status: other,
        })),
    }
}
