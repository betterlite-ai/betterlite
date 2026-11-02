use std::collections::BTreeMap;

use crate::error::details::{
    ApiDetails, RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ProviderErrorFacts, bounded_text};
use crate::parse::get_error_message;
use crate::rate_limit::RateLimitCategory;

pub mod body;
mod phrase;
mod tests;

use body::{OpenaiErrorBody, typed_body};
use phrase::{phrase_error_class, phrase_family};

// mirrors the _map_openai_exception kwargs read by its branch predicates and raised exceptions
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenaiExceptionInput {
    pub model: String,
    pub custom_llm_provider: String,
    pub error_str: String,
    pub status_code: Option<u16>,
    #[serde(default)]
    pub status_code_is_synthesized: bool,
    pub message: Option<String>,
    #[serde(default)]
    pub body: Option<OpenaiErrorBody>,
}

#[must_use]
pub fn map_openai_exception(input: &OpenaiExceptionInput) -> ErrorClass {
    let provider = exception_provider(&input.custom_llm_provider);
    let message = resolve_message(input);
    phrase_family(input).map_or_else(
        || status_branch(input, &provider, &message),
        |family| phrase_error_class(input, &provider, &message, family),
    )
}

// custom_llm_provider == "openai" becomes "OpenAIException"; every other provider is Title-cased plus "Exception"
fn exception_provider(custom_llm_provider: &str) -> String {
    if custom_llm_provider == "openai" {
        return "OpenAIException".to_owned();
    }
    let mut chars = custom_llm_provider.chars();
    chars.next().map_or_else(
        || "Exception".to_owned(),
        |first| format!("{}{}Exception", first.to_uppercase(), chars.as_str()),
    )
}

// mirrors get_error_message plus _map_openai_exception's OPENAI/openai.OpenAIError substitutions
fn resolve_message(input: &OpenaiExceptionInput) -> String {
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
    let provider_upper = input.custom_llm_provider.to_uppercase();
    let replaced = resolved.replace("OPENAI", &provider_upper).replace(
        "openai.OpenAIError",
        &format!("{0}.{0}Error", input.custom_llm_provider),
    );
    bounded_text(&replaced)
}

type BodyFields = (Option<String>, Option<String>, Option<String>);
const NO_BODY: BodyFields = (None, None, None);

// openai.APIError reads type/param/code straight off the flat body, unlike Azure's nested shape
fn flat_body(input: &OpenaiExceptionInput) -> BodyFields {
    let fields = input.body.as_ref();
    (
        fields.and_then(|body| body.kind.clone()),
        fields.and_then(|body| body.param.clone()),
        fields.and_then(|body| body.code.clone()),
    )
}

fn openai_context(input: &OpenaiExceptionInput, message: String, body: BodyFields) -> ErrorContext {
    let (body_type, body_param, body_code) = body;
    ErrorContext {
        message,
        model: Some(input.model.clone()),
        provider: Some(input.custom_llm_provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type,
        body_param,
        body_code,
    }
}

const STATUS_LABELS: &[(u16, &str)] = &[
    (400, ""),
    (401, "AuthenticationError: "),
    (404, "NotFoundError: "),
    (408, "Timeout Error: "),
    (422, ""),
    (429, "RateLimitError: "),
    (500, "InternalServerError: "),
    (502, "BadGatewayError: "),
    (503, "ServiceUnavailableError: "),
    (504, "Timeout Error: "),
];

fn status_text(provider: &str, status: Option<u16>, message: &str) -> String {
    let Some(code) = status else {
        return format!("APIConnectionError: {provider} - {message}");
    };
    let label = STATUS_LABELS
        .iter()
        .find(|(known, _)| *known == code)
        .map_or("APIError: ", |(_, label)| *label);
    format!("{label}{provider} - {message}")
}

// the status-code fallback _map_openai_exception reaches once every error-string predicate above misses
fn status_branch(input: &OpenaiExceptionInput, provider: &str, message: &str) -> ErrorClass {
    // hasattr(original_exception, "status_code") is false for a synthesized status, so it must read as None here too
    let status = if input.status_code_is_synthesized {
        None
    } else {
        input.status_code
    };
    let text = status_text(provider, status, message);
    let body = if matches!(status, Some(400 | 422 | 429 | 500)) {
        flat_body(input)
    } else {
        NO_BODY
    };
    let context = openai_context(input, text, body);
    status_class(status, context)
}

fn status_class(status: Option<u16>, context: ErrorContext) -> ErrorClass {
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
        Some(500) => ErrorClass::InternalServer(Box::new(context)),
        Some(502) => ErrorClass::BadGateway(Box::new(context)),
        Some(503) => ErrorClass::ServiceUnavailable(Box::new(context)),
        // the only Timeout raise that forwards the real upstream status rather than defaulting to 408
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
