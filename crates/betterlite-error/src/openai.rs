use std::collections::BTreeMap;

use crate::error::details::{ApiDetails, RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ExceptionCheckers, ProviderErrorFacts, bounded_text};
use crate::parse::get_error_message;
use crate::rate_limit::RateLimitCategory;

pub mod body;
mod tests;

use body::{OpenaiErrorBody, typed_body};

// mirrors the _map_openai_exception kwargs read by its branch predicates and raised exceptions
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenaiExceptionInput {
    pub model: String,
    pub custom_llm_provider: String,
    pub error_str: String,
    pub status_code: Option<u16>,
    pub message: Option<String>,
    #[serde(default)]
    pub body: Option<OpenaiErrorBody>,
}

#[must_use]
pub fn map_openai_exception(input: &OpenaiExceptionInput) -> ErrorClass {
    let provider = exception_provider(&input.custom_llm_provider);
    let message = resolve_message(input);
    rate_limit_branch(input, &provider, &message)
        .or_else(|| context_window_branch(input, &provider, &message))
        .or_else(|| model_not_found_branch(input, &provider, &message))
        .or_else(|| timeout_string_branch(input, &provider, &message))
        .or_else(|| content_policy_branch(input, &provider, &message))
        .or_else(|| encrypted_content_branch(input, &provider, &message))
        .or_else(|| invalid_request_branch(input, &provider, &message))
        .or_else(|| internal_server_text_branch(input, &provider, &message))
        .or_else(|| request_too_large_branch(input, &provider, &message))
        .or_else(|| missing_credential_branch(input, &provider, &message))
        .or_else(|| mistral_streaming_branch(input, &provider, &message))
        .unwrap_or_else(|| status_branch(input, &provider, &message))
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

fn rate_limit_error(input: &OpenaiExceptionInput, text: String, body: BodyFields) -> ErrorClass {
    let context = openai_context(input, text, body);
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context,
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

fn rate_limit_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    if !ExceptionCheckers::is_error_str_rate_limit(&input.error_str, input.status_code) {
        return None;
    }
    let text = format!("RateLimitError: {provider} - {message}");
    Some(rate_limit_error(input, text, flat_body(input)))
}

fn context_window_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    if !ExceptionCheckers::context_window(&input.error_str) {
        return None;
    }
    let text = format!("ContextWindowExceededError: {provider} - {message}");
    let context = openai_context(input, text, NO_BODY);
    Some(ErrorClass::ContextWindowExceeded(Box::new(context)))
}

fn model_not_found_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    let matched = input.error_str.contains("invalid_request_error")
        && input.error_str.contains("model_not_found");
    if !matched {
        return None;
    }
    let text = format!("{provider} - {message}");
    let context = openai_context(input, text, NO_BODY);
    Some(ErrorClass::NotFound(Box::new(context)))
}

fn timeout_string_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    if !input.error_str.contains("A timeout occurred") {
        return None;
    }
    let text = format!("{provider} - {message}");
    let context = openai_context(input, text, NO_BODY);
    Some(ErrorClass::Timeout(Box::new(TimeoutDetails {
        context,
        original_status: TimeoutStatus(408),
    })))
}

fn content_policy_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    let matched = (input.error_str.contains("invalid_request_error")
        && input.error_str.contains("content_policy_violation"))
        || (input.error_str.contains("Invalid prompt")
            && input.error_str.contains("violating our usage policy"))
        || input
            .error_str
            .to_lowercase()
            .contains("request was rejected as a result of the safety system");
    if !matched {
        return None;
    }
    let text = format!("ContentPolicyViolationError: {provider} - {message}");
    let context = openai_context(input, text, flat_body(input));
    Some(ErrorClass::ContentPolicyViolation(Box::new(context)))
}

fn encrypted_content_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    let matched = input.error_str.contains("invalid_encrypted_content")
        || input.error_str.contains("could not be verified");
    if !matched {
        return None;
    }
    let text = format!(
        "{provider} - {message}\n\nEncrypted content affinity is required across deployments."
    );
    let context = openai_context(input, text, flat_body(input));
    Some(ErrorClass::BadRequest(Box::new(context)))
}

fn invalid_request_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    let matched = input.error_str.contains("invalid_request_error")
        && !input.error_str.contains("Incorrect API key provided");
    if !matched {
        return None;
    }
    let text = format!("{provider} - {message}");
    let context = openai_context(input, text, flat_body(input));
    Some(ErrorClass::BadRequest(Box::new(context)))
}

fn internal_server_text_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    let matched = input
        .error_str
        .contains("Web server is returning an unknown error")
        || input
            .error_str
            .contains("The server had an error processing your request.");
    if !matched {
        return None;
    }
    let text = format!("{provider} - {message}");
    let context = openai_context(input, text, flat_body(input));
    Some(ErrorClass::InternalServer(Box::new(context)))
}

fn request_too_large_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    if !input.error_str.contains("Request too large") {
        return None;
    }
    let text = format!("RateLimitError: {provider} - {message}");
    Some(rate_limit_error(input, text, flat_body(input)))
}

const MISSING_CREDENTIAL: &str = "The api_key client option must be set either by passing api_key \
to the client or by setting the OPENAI_API_KEY environment variable";

fn missing_credential_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    if !input.error_str.contains(MISSING_CREDENTIAL) {
        return None;
    }
    let text = format!("AuthenticationError: {provider} - {message}");
    let context = openai_context(input, text, NO_BODY);
    Some(ErrorClass::Authentication(Box::new(context)))
}

fn mistral_streaming_branch(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
) -> Option<ErrorClass> {
    if !input
        .error_str
        .contains("Mistral API raised a streaming error")
    {
        return None;
    }
    let text = format!("{provider} - {message}");
    let context = openai_context(input, text, NO_BODY);
    Some(ErrorClass::Api(Box::new(ApiDetails {
        context,
        status: 500,
    })))
}

fn status_text(provider: &str, status: Option<u16>, message: &str) -> String {
    match status {
        None => format!("APIConnectionError: {provider} - {message}"),
        Some(400 | 422) => format!("{provider} - {message}"),
        Some(401) => format!("AuthenticationError: {provider} - {message}"),
        Some(404) => format!("NotFoundError: {provider} - {message}"),
        Some(408 | 504) => format!("Timeout Error: {provider} - {message}"),
        Some(429) => format!("RateLimitError: {provider} - {message}"),
        Some(500) => format!("InternalServerError: {provider} - {message}"),
        Some(502) => format!("BadGatewayError: {provider} - {message}"),
        Some(503) => format!("ServiceUnavailableError: {provider} - {message}"),
        Some(_) => format!("APIError: {provider} - {message}"),
    }
}

// the status-code fallback _map_openai_exception reaches once every error-string predicate above misses
fn status_branch(input: &OpenaiExceptionInput, provider: &str, message: &str) -> ErrorClass {
    let status = input.status_code;
    let text = status_text(provider, status, message);
    let body = if matches!(status, Some(400 | 422 | 429 | 500)) {
        flat_body(input)
    } else {
        NO_BODY
    };
    let context = openai_context(input, text, body);
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
