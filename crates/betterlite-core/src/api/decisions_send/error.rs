use std::error::Error;
use std::time::Duration;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::{TimeoutDetails, TimeoutStatus};
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_http::interactions::base::RequestFailure;
use betterlite_models::decisions::Json;
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::openai::response::ResponseConversionError;
use betterlite_openai::decisions::transformation::provider_config::{
    DecisionsProviderConfig, ParseResponseError,
};
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::vector_store_http_write::error::{
    BaseLlmException, FailureFacts, HeaderMap as JsonHeaders, display_facts, handle_error,
    timeout_facts,
};
use crate::api::vector_store_http_write::service::ClientKind;

const TIMEOUT_PREFIX: &str = "litellm.Timeout: ";

const TIMEOUT_MODEL: &str = "default-model-name";
const TIMEOUT_PROVIDER: &str = "litellm-httpx-handler";

#[derive(Debug, Error)]
pub enum DecisionsSendError {
    #[error("{message}")]
    Mapped {
        class: ErrorClass,
        message: String,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    // _format_response runs outside send's try block, so its conversion error escapes unmapped
    #[error(transparent)]
    Format(#[from] ResponseConversionError),
}

impl DecisionsSendError {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Mapped { message, .. } => message.clone(),
            Self::Format(error) => error.to_string(),
        }
    }
}

// The text the SDK reports: each LiteLLM exception's __init__ prefixes the message with its class name
#[must_use]
pub fn visible_message(class: &ErrorClass) -> String {
    let body = class_message(class);
    match class {
        ErrorClass::Authentication(_) => format!("litellm.AuthenticationError: {body}"),
        ErrorClass::NotFound(_) => format!("litellm.NotFoundError: {body}"),
        ErrorClass::PermissionDenied(_) => format!("litellm.PermissionDeniedError: {body}"),
        ErrorClass::BadRequest(_) => format!("litellm.BadRequestError: {body}"),
        ErrorClass::UnprocessableEntity(_) => format!("litellm.UnprocessableEntityError: {body}"),
        ErrorClass::Timeout(_) => format!("{TIMEOUT_PREFIX}{body}"),
        ErrorClass::RateLimit(_) => format!("litellm.RateLimitError: {body}"),
        ErrorClass::ServiceUnavailable(_) => format!("litellm.ServiceUnavailableError: {body}"),
        ErrorClass::BadGateway(_) => format!("litellm.BadGatewayError: {body}"),
        ErrorClass::InternalServer(_) => format!("litellm.InternalServerError: {body}"),
        ErrorClass::Api(_) => format!("litellm.APIError: {body}"),
        ErrorClass::ApiConnection(_) => format!("litellm.APIConnectionError: {body}"),
        ErrorClass::ApiResponseValidation(_) => {
            format!("litellm.APIResponseValidationError: {body}")
        }
        ErrorClass::ContextWindowExceeded(_) => {
            format!("litellm.ContextWindowExceededError: {body}")
        }
        ErrorClass::ContentPolicyViolation(_) => {
            format!("litellm.ContentPolicyViolationError: {body}")
        }
        ErrorClass::UnsupportedParams(_) => format!("litellm.UnsupportedParamsError: {body}"),
        ErrorClass::MidStreamFallback(_) => format!("litellm.MidStreamFallbackError: {body}"),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::Unclassified(_) => body,
    }
}

// Every entry into exception_type names the model, the provider and the error the same way
pub(super) fn raw_exception(
    call: &DecisionsCall<'_>,
    error_str: String,
    status_code: Option<u16>,
    synthesized: bool,
    message: Option<String>,
    exception_type: &str,
) -> RawException {
    RawException {
        model: qualified_model(call),
        custom_llm_provider: call.custom_llm_provider.clone(),
        error_str,
        status_code,
        status_code_is_synthesized: synthesized,
        message,
        body: None,
        exception_type: exception_type.to_owned(),
    }
}

// A synthesized status takes _map_upstream_exception's direct APIConnectionError, anything else exception_type's mapping
pub(crate) fn provider_error(
    call: &DecisionsCall<'_>,
    redactor: &Redactor,
    error: BaseLlmException,
) -> DecisionsSendError {
    if error.status_code_is_synthesized {
        return synthesized(call, error);
    }
    let message = error.message.clone();
    let raw = raw_exception(
        call,
        message.clone(),
        Some(error.status_code),
        false,
        Some(message),
        "BaseLLMException",
    );
    let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
    mapped(class, Box::new(error))
}

// A wrong-shape answer raises a 500 BaseLLMException, while JSON that does not parse escapes as a ValidationError
pub(crate) fn transform_error(
    call: &DecisionsCall<'_>,
    redactor: &Redactor,
    error: ParseResponseError,
) -> DecisionsSendError {
    match error {
        ParseResponseError::Invalid(provider) => {
            provider_error(call, redactor, base_exception(*provider, false))
        }
        ParseResponseError::Malformed(source) => {
            let text = source.to_string();
            let raw = raw_exception(call, text, None, false, None, "ValidationError");
            let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
            mapped(class, Box::new(ParseResponseError::Malformed(source)))
        }
    }
}

// A failed send: a deadline is already litellm.Timeout from the client, and every other httpx.HTTPError goes through _handle_error
pub(crate) fn send_failure<T>(
    config: &T,
    call: &DecisionsCall<'_>,
    redactor: &Redactor,
    error: RequestFailure,
    mode: ClientKind,
    elapsed: Duration,
) -> DecisionsSendError
where
    T: DecisionsProviderConfig,
{
    if let RequestFailure::Transport(source) = &error
        && source.is_timeout()
    {
        return handler_timeout(call.timeout, mode, elapsed, error);
    }
    mapped_failure(config, call, redactor, &display_facts(error.to_string()))
}

// _handle_error with a provider config: B2402's conversion picks the status and masked headers, then get_error_class raises the provider's own error
pub(crate) fn mapped_failure<T>(
    config: &T,
    call: &DecisionsCall<'_>,
    redactor: &Redactor,
    facts: &FailureFacts,
) -> DecisionsSendError
where
    T: DecisionsProviderConfig,
{
    let converted = handle_error(facts, None);
    let provider = config.get_error_class(
        &converted.message,
        converted.status_code,
        &provider_headers(&converted.headers),
    );
    provider_error(
        call,
        redactor,
        base_exception(provider, converted.status_code_is_synthesized),
    )
}

// The caught exception's headers reach get_error_class as strings; the conversion only builds text values
fn provider_headers(headers: &JsonHeaders) -> Headers {
    headers
        .iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            _ => None,
        })
        .collect()
}

// ProviderError.headers is the class's optional list of pairs, and the shared conversion reads a map of text
fn error_headers(headers: Option<Vec<(String, String)>>) -> JsonHeaders {
    headers.map_or_else(JsonHeaders::new, |entries| {
        entries
            .into_iter()
            .map(|(name, value)| (name, Json::Text(value)))
            .collect()
    })
}

// The provider's own error class as the BaseLLMException exception_type reads, plus a status the transport step made up
fn base_exception(provider: ProviderError, or_synthesized: bool) -> BaseLlmException {
    BaseLlmException {
        status_code: provider.status_code,
        message: provider.message,
        headers: error_headers(provider.headers),
        status_code_is_synthesized: provider.status_code_is_synthesized || or_synthesized,
    }
}

// litellm.Timeout keeps the handler's message and its own 408, and exception_type returns it unchanged
fn handler_timeout(
    timeout: Option<Duration>,
    mode: ClientKind,
    elapsed: Duration,
    failure: RequestFailure,
) -> DecisionsSendError {
    let facts = timeout_facts(mode, timeout, elapsed);
    let visible = facts.text.unwrap_or_default();
    let constructor = visible
        .strip_prefix(TIMEOUT_PREFIX)
        .unwrap_or(&visible)
        .to_owned();
    let context = ErrorContext {
        message: constructor,
        model: Some(String::from(TIMEOUT_MODEL)),
        provider: Some(String::from(TIMEOUT_PROVIDER)),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    let class = ErrorClass::Timeout(Box::new(TimeoutDetails {
        context,
        original_status: TimeoutStatus(408),
    }));
    mapped(class, Box::new(failure))
}

// litellm.APIConnectionError(message=f"{provider_label} - {error.message}", ...), raised directly
fn synthesized(call: &DecisionsCall<'_>, error: BaseLlmException) -> DecisionsSendError {
    let context = ErrorContext {
        message: format!(
            "{} - {}",
            provider_label(&call.custom_llm_provider),
            error.message
        ),
        model: Some(qualified_model(call)),
        provider: Some(call.custom_llm_provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    let class = ErrorClass::ApiConnection(Box::new(context));
    mapped(class, Box::new(error))
}

pub(super) fn mapped(
    class: ErrorClass,
    source: Box<dyn Error + Send + Sync>,
) -> DecisionsSendError {
    let message = visible_message(&class);
    DecisionsSendError::Mapped {
        class,
        message,
        source,
    }
}

// ast-grep-ignore: no-one-line-functions - six exception paths name the provider and model the same way
fn qualified_model(call: &DecisionsCall<'_>) -> String {
    format!("{}/{}", call.custom_llm_provider, call.model)
}

// custom_llm_provider[0].upper() + custom_llm_provider[1:] + "Exception"
fn provider_label(provider: &str) -> String {
    let mut chars = provider.chars();
    chars.next().map_or_else(
        || String::from("Exception"),
        |first| format!("{}{}Exception", first.to_uppercase(), chars.as_str()),
    )
}
