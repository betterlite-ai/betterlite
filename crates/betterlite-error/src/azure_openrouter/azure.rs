use std::collections::BTreeMap;

use crate::error::details::{ApiDetails, RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ExceptionCheckers, ProviderErrorFacts, bounded_text};
use crate::parse::get_error_message;
use crate::rate_limit::RateLimitCategory;

pub mod body;
mod tests;

use body::{AzureErrorBody, azure_error_code, content_policy_fields, typed_body};

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

type ErrorClassBuilder = fn(Box<ErrorContext>) -> ErrorClass;
type TextPredicate = fn(&str) -> bool;

// predicate to test error_str against, the label to show in the rendered message, and the variant it maps to
const TEXT_CANDIDATES: [(TextPredicate, &str, ErrorClassBuilder); 3] = [
    (
        |error_str| error_str.contains("Internal server error"),
        "Internal server error",
        ErrorClass::InternalServer,
    ),
    (
        |error_str| error_str.contains("This model's maximum context length is"),
        "ContextWindowExceededError",
        ErrorClass::ContextWindowExceeded,
    ),
    (
        |error_str| error_str.contains("DeploymentNotFound"),
        "NotFoundError",
        ErrorClass::NotFound,
    ),
];

fn text_classification(input: &AzureExceptionInput, message: &str) -> Option<ErrorClass> {
    let found = TEXT_CANDIDATES
        .into_iter()
        .find(|(matches, ..)| matches(&input.error_str));
    found.map(|(_, label, build)| {
        let text = format!("AzureException {label} - {message}");
        let context = azure_context(input, "azure", text, NO_BODY);
        build(Box::new(context))
    })
}

fn content_policy_classification(input: &AzureExceptionInput, message: &str) -> Option<ErrorClass> {
    let matched = azure_error_code(input.body.as_ref()).as_deref()
        == Some("content_policy_violation")
        || ExceptionCheckers::azure_policy(&input.error_str);
    if !matched {
        return None;
    }
    let (error_type, error_code) = content_policy_fields(input.body.as_ref());
    let text = format!("litellm.BadRequestError: litellm.ContentPolicyViolationError: {message}");
    let body = (
        Some(error_type.unwrap_or_else(|| "invalid_request_error".to_owned())),
        None,
        Some(error_code.unwrap_or_else(|| "content_policy_violation".to_owned())),
    );
    let context = azure_context(input, "azure", text, body);
    Some(ErrorClass::ContentPolicyViolation(Box::new(context)))
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
        return Some(ErrorClass::BadRequest(Box::new(azure_context(
            input, "azure", text, body,
        ))));
    }
    let is_invalid_request = input.error_str.contains("invalid_request_error")
        && matches!(input.status_code, None | Some(400));
    if is_invalid_request {
        let text = format!("AzureException BadRequestError - {message}");
        return Some(ErrorClass::BadRequest(Box::new(azure_context(
            input, "azure", text, body,
        ))));
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
        return Some(ErrorClass::Authentication(Box::new(context)));
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

fn status_text(
    input: &AzureExceptionInput,
    status: Option<u16>,
    message: &str,
) -> (String, BodyFields) {
    match status {
        None => (
            format!(
                "{} APIConnectionError - {message}",
                input.exception_provider
            ),
            (None, None, None),
        ),
        Some(400) => {
            let fields = input.body.as_ref();
            (
                format!("AzureException - {message}"),
                (
                    fields.and_then(|body| body.kind.clone()),
                    fields.and_then(|body| body.param.clone()),
                    fields.and_then(|body| body.code.clone()),
                ),
            )
        }
        Some(other) => {
            let label = match other {
                401 => "AuthenticationError",
                404 => "NotFoundError",
                408 | 504 => "Timeout",
                422 => "BadRequestError",
                429 => "RateLimitError",
                502 => "BadGatewayError",
                503 => "ServiceUnavailableError",
                _ => "APIError",
            };
            (
                format!("AzureException {label} - {message}"),
                (None, None, None),
            )
        }
    }
}

fn status_classification(input: &AzureExceptionInput, message: &str) -> ErrorClass {
    let status = input.status_code;
    let (text, body) = status_text(input, status, message);
    let context = azure_context(input, "azure", text, body);
    match status {
        None => ErrorClass::ApiConnection(Box::new(context)),
        Some(400 | 422) => ErrorClass::BadRequest(Box::new(context)),
        Some(401) => ErrorClass::Authentication(Box::new(context)),
        Some(404) => ErrorClass::NotFound(Box::new(context)),
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
