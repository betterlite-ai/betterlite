use std::collections::BTreeMap;

use crate::error::details::{
    ApiDetails, RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ProviderErrorFacts, bounded_text};
use crate::parse::get_error_message;
use crate::rate_limit::RateLimitCategory;

pub mod body;
mod tests;

use body::{
    AzureErrorBody, azure_error_code, content_policy_context, is_content_policy, typed_body,
};

// mirrors the _map_azure_exception kwargs read by its branch predicates and raised exceptions
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AzureExceptionInput {
    pub model: String,
    pub custom_llm_provider: String,
    pub error_str: String,
    pub exception_provider: String,
    pub status_code: Option<u16>,
    pub message: Option<String>,
    #[serde(default)]
    pub body: Option<AzureErrorBody>,
}

#[must_use]
pub fn map_azure_exception(input: &AzureExceptionInput) -> ErrorClass {
    let message = resolve_message(input);
    text_classification(input, &message)
        .or_else(|| content_policy_classification(input, &message))
        .or_else(|| encrypted_or_invalid_request(input, &message))
        .or_else(|| credential_or_connection(input, &message))
        .unwrap_or_else(|| status_classification(input, &message))
}

// get_error_message(original_exception), falling back to .message then str(original_exception)
fn resolve_message(input: &AzureExceptionInput) -> String {
    let facts = ProviderErrorFacts {
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: typed_body(input.body.as_ref()),
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    let resolved = get_error_message(&facts)
        .or_else(|| input.message.clone())
        .unwrap_or_else(|| input.error_str.clone());
    bounded_text(&resolved)
}

type BodyFields = (Option<String>, Option<String>, Option<String>);

const NO_BODY: BodyFields = (None, None, None);

fn azure_context(
    input: &AzureExceptionInput,
    provider: &str,
    message: String,
    body: BodyFields,
) -> ErrorContext {
    let (body_type, body_param, body_code) = body;
    ErrorContext {
        message,
        model: Some(input.model.clone()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type,
        body_param,
        body_code,
    }
}

type TextPredicate = fn(&str) -> bool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextFamily {
    InternalServer,
    ContextWindowExceeded,
    NotFound,
}

// predicate to test error_str against, the label to show in the rendered message, and the family it maps to
const TEXT_CANDIDATES: [(TextPredicate, &str, TextFamily); 3] = [
    (
        |error_str| error_str.contains("Internal server error"),
        "Internal server error",
        TextFamily::InternalServer,
    ),
    (
        |error_str| error_str.contains("This model's maximum context length is"),
        "ContextWindowExceededError",
        TextFamily::ContextWindowExceeded,
    ),
    (
        |error_str| error_str.contains("DeploymentNotFound"),
        "NotFoundError",
        TextFamily::NotFound,
    ),
];

// _map_azure_exception passes a response on both the context-window and DeploymentNotFound branches
fn text_classification(input: &AzureExceptionInput, message: &str) -> Option<ErrorClass> {
    let found = TEXT_CANDIDATES
        .into_iter()
        .find(|(matches, ..)| matches(&input.error_str));
    found.map(|(_, label, family)| {
        let text = format!("AzureException {label} - {message}");
        let context = azure_context(input, "azure", text, NO_BODY);
        match family {
            TextFamily::InternalServer => ErrorClass::InternalServer(Box::new(context)),
            TextFamily::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(
                ResponseDetails::with_status(context, input.status_code),
            )),
            TextFamily::NotFound => ErrorClass::NotFound(Box::new(ResponseDetails::with_status(
                context,
                input.status_code,
            ))),
        }
    })
}

fn content_policy_classification(input: &AzureExceptionInput, message: &str) -> Option<ErrorClass> {
    if !is_content_policy(input) {
        return None;
    }
    let context = content_policy_context(input, message);
    Some(ErrorClass::ContentPolicyViolation(Box::new(
        ResponseDetails::with_status(context, input.status_code),
    )))
}

fn encrypted_or_invalid_request(input: &AzureExceptionInput, message: &str) -> Option<ErrorClass> {
    let body = (
        input.body.as_ref().and_then(|fields| fields.kind.clone()),
        input.body.as_ref().and_then(|fields| fields.param.clone()),
        input.body.as_ref().and_then(|fields| fields.code.clone()),
    );
    let is_encrypted = azure_error_code(input.body.as_ref()).as_deref()
        == Some("invalid_encrypted_content")
        || input.error_str.contains("could not be verified");
    if is_encrypted {
        let text = format!(
            "AzureException - {message}\n\nEncrypted content affinity is required across deployments."
        );
        return Some(ErrorClass::BadRequest(Box::new(
            ResponseDetails::with_status(
                azure_context(input, "azure", text, body),
                input.status_code,
            ),
        )));
    }
    let is_invalid_request = input.error_str.contains("invalid_request_error")
        && matches!(input.status_code, None | Some(400));
    if is_invalid_request {
        let text = format!("AzureException BadRequestError - {message}");
        return Some(ErrorClass::BadRequest(Box::new(
            ResponseDetails::with_status(
                azure_context(input, "azure", text, body),
                input.status_code,
            ),
        )));
    }
    None
}

const MISSING_CREDENTIAL: &str =
    "The api_key client option must be set either by passing api_key to the client or by setting";

fn credential_or_connection(input: &AzureExceptionInput, message: &str) -> Option<ErrorClass> {
    let provider = input.custom_llm_provider.clone();
    if input.error_str.contains(MISSING_CREDENTIAL) {
        let text = format!(
            "{} AuthenticationError - {message}",
            input.exception_provider
        );
        let context = azure_context(input, &provider, text, NO_BODY);
        return Some(ErrorClass::Authentication(Box::new(
            ResponseDetails::with_status(context, input.status_code),
        )));
    }
    if input.error_str.contains("Connection error") {
        let text = format!(
            "{} APIConnectionError - {message}",
            input.exception_provider
        );
        let context = azure_context(input, &provider, text, NO_BODY);
        return Some(ErrorClass::ApiConnection(Box::new(context)));
    }
    None
}

const STATUS_LABELS: &[(u16, &str)] = &[
    (401, "AuthenticationError"),
    (404, "NotFoundError"),
    (408, "Timeout"),
    (422, "BadRequestError"),
    (429, "RateLimitError"),
    (502, "BadGatewayError"),
    (503, "ServiceUnavailableError"),
    (504, "Timeout"),
];

fn status_text(
    input: &AzureExceptionInput,
    status: Option<u16>,
    message: &str,
) -> (String, BodyFields) {
    let Some(code) = status else {
        let text = format!(
            "{} APIConnectionError - {message}",
            input.exception_provider
        );
        return (text, (None, None, None));
    };
    if code == 400 {
        let fields = input.body.as_ref();
        let body = (
            fields.and_then(|body| body.kind.clone()),
            fields.and_then(|body| body.param.clone()),
            fields.and_then(|body| body.code.clone()),
        );
        return (format!("AzureException - {message}"), body);
    }
    let label = STATUS_LABELS
        .iter()
        .find(|(known, _)| *known == code)
        .map_or("APIError", |(_, label)| *label);
    (
        format!("AzureException {label} - {message}"),
        (None, None, None),
    )
}

fn status_classification(input: &AzureExceptionInput, message: &str) -> ErrorClass {
    let status = input.status_code;
    let (text, body) = status_text(input, status, message);
    let context = azure_context(input, "azure", text, body);
    match status {
        None => ErrorClass::ApiConnection(Box::new(context)),
        Some(code @ (400 | 422)) => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, Some(code))))
        }
        Some(code @ 401) => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, Some(code))))
        }
        Some(code @ 404) => {
            ErrorClass::NotFound(Box::new(ResponseDetails::with_status(context, Some(code))))
        }
        Some(408) => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408),
        })),
        Some(429) => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Some(502) => ErrorClass::BadGateway(Box::new(context)),
        Some(503) => ErrorClass::ServiceUnavailable(Box::new(context)),
        Some(504) => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(504),
        })),
        Some(other) => ErrorClass::Api(Box::new(ApiDetails {
            context,
            status: other,
        })),
    }
}
