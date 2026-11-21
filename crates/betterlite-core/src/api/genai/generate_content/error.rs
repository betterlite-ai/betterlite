use std::error::Error;
use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::genai::chat_call::ChatCallError;
use crate::api::genai_http::error::GenAiHttpError;
use crate::api::vector_store_http_write::error::BaseLlmException;
use crate::genai_setup::error::GenAiSetupError;
use crate::genai_setup::params::RequestFieldError;

// timeout or request_timeout: REQUEST_TIMEOUT has to be a negative-free number of seconds to become a deadline
#[derive(Debug, Error)]
#[error("the request timeout of {seconds} seconds is not a positive duration: {source}")]
pub struct TimeoutError {
    pub(super) seconds: f64,
    #[source]
    pub(super) source: TryFromFloatSecsError,
}

// what one of the four entry points raises before its exception_type call maps the class
#[derive(Debug, Error)]
pub enum GenerateContentError<Source, ExecutorSource>
where
    Source: Error + 'static,
    ExecutorSource: Error + 'static,
{
    #[error(transparent)]
    Fields(#[from] RequestFieldError),
    #[error(transparent)]
    Resolve(#[from] ResolutionError),
    #[error(transparent)]
    Setup(#[from] GenAiSetupError<Source>),
    #[error(transparent)]
    Adapter(#[from] ChatCallError<ExecutorSource>),
    #[error(transparent)]
    Native(#[from] GenAiHttpError<Source>),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
}

// the mapped class next to the local error chain, the way every other module reports a LiteLLM exception
#[derive(Debug, Error)]
#[error("{error}")]
pub struct GenerateContentFailure<Source, ExecutorSource>
where
    Source: Error + 'static,
    ExecutorSource: Error + 'static,
{
    #[source]
    pub error: GenerateContentError<Source, ExecutorSource>,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=model, custom_llm_provider=provider, original_exception=...)
#[must_use]
pub fn failure<Source, ExecutorSource>(
    error: GenerateContentError<Source, ExecutorSource>,
    model: &str,
    provider: Option<&str>,
    redactor: &Redactor,
) -> GenerateContentFailure<Source, ExecutorSource>
where
    Source: Error + 'static,
    ExecutorSource: Error + 'static,
{
    let name = provider.unwrap_or_default();
    let message = error.to_string();
    let class = match &error {
        GenerateContentError::Fields(field) => named_field_class(message, field.field, model, name),
        // get_llm_provider raises its own 400 before the call reaches any provider
        GenerateContentError::Resolve(source)
        | GenerateContentError::Setup(GenAiSetupError::Resolution(source)) => {
            let raw = exception_raw(source.to_string(), model, name, Some(400));
            dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
        }
        GenerateContentError::Setup(GenAiSetupError::RequestField(field)) => {
            named_field_class(message, field.field, model, name)
        }
        GenerateContentError::Setup(
            GenAiSetupError::MockResponseHandledByCaller
            | GenAiSetupError::LoggingRequired
            | GenAiSetupError::Logging(_)
            | GenAiSetupError::Transform(_),
        ) => value_error(message, model, name, redactor),
        GenerateContentError::Adapter(_) => value_error(message, model, name, redactor),
        GenerateContentError::Native(GenAiHttpError::Provider(exception)) => {
            provider_class(exception, model, name, redactor)
        }
        GenerateContentError::Native(
            GenAiHttpError::Client(_)
            | GenAiHttpError::Logging(_)
            | GenAiHttpError::Auth(_)
            | GenAiHttpError::Transform(_)
            | GenAiHttpError::BodyNotObject
            | GenAiHttpError::UnexpectedStream,
        ) => value_error(message, model, name, redactor),
        GenerateContentError::Timeout(_) => value_error(message, model, name, redactor),
    };
    GenerateContentFailure { error, class }
}

// a BaseLLMException keeps the status the provider answered, synthesized or not
fn provider_class(
    exception: &BaseLlmException,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> ErrorClass {
    let mut raw = exception_raw(
        exception.message.clone(),
        model,
        provider,
        Some(exception.status_code),
    );
    raw.status_code_is_synthesized = exception.status_code_is_synthesized;
    "BaseLLMException".clone_into(&mut raw.exception_type);
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// every other local failure reaches exception_type as a ValueError with no status of its own
fn value_error(message: String, model: &str, provider: &str, redactor: &Redactor) -> ErrorClass {
    let mut raw = exception_raw(message, model, provider, None);
    "ValueError".clone_into(&mut raw.exception_type);
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// docs/compatibility.md rejects a value outside its declared type with a 400 that names the field
fn named_field_class(message: String, field: &str, model: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message,
        model: Some(model.to_owned()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

// the attributes exception_type reads off the original exception before any provider branch runs
fn exception_raw(
    message: String,
    model: &str,
    provider: &str,
    status_code: Option<u16>,
) -> RawException {
    RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
