use std::error::Error as StdError;
use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::video_selection::error::VideoSelectionError;

// exception_type() maps every create and get failure with model "" and the caller's custom_llm_provider
const MODEL: &str = "";

// LiteLLM's json.loads(mock_response) then CharacterObject(**mock_response)
#[derive(Debug, Error)]
pub enum MockDecodeError {
    #[error("the mock response is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the mock response does not match {expected}")]
    Model {
        expected: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

// what video_create_character and video_get_character raise before exception_type sees it
#[derive(Debug, Error)]
pub enum CharacterCallError {
    #[error(transparent)]
    Mock(#[from] MockDecodeError),
    // LlmProviders(custom_llm_provider) rejects a name outside the provider set
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    // LiteLLM's ValueError when get_provider_video_config answers None
    #[error("video create character is not supported for {provider}")]
    CreateUnsupported { provider: String },
    #[error("video get character is not supported for {provider}")]
    GetUnsupported { provider: String },
    // B3332's selector raises this for a provider whose config a later task installs
    #[error(transparent)]
    Selection(#[from] VideoSelectionError),
    #[error("the request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // docs/compatibility.md rejects a keyword value outside its declared type
    #[error("{field} is not a string")]
    FieldType { field: &'static str },
    #[error(transparent)]
    Handler(#[from] CharacterHandlerFailure),
}

// a provider error class B3335's transport raised, before exception_type maps it
pub trait CharacterProviderFailure: StdError + Send + Sync + 'static {
    fn status_code(&self) -> Option<u16>;
    fn message(&self) -> String;
}

// what a handler answered with when it did not answer with the character itself
#[derive(Debug, Error)]
#[error("{message}")]
pub struct CharacterHandlerFailure {
    pub status_code: Option<u16>,
    pub message: String,
    #[source]
    pub source: Box<dyn StdError + Send + Sync>,
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct CharacterCallFailure {
    #[source]
    pub error: CharacterCallError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...), run once
#[must_use]
pub fn failure(
    error: CharacterCallError,
    provider: &str,
    redactor: &Redactor,
) -> CharacterCallFailure {
    let class = match &error {
        CharacterCallError::FieldType { field } => {
            named_field_class(error.to_string(), field, provider)
        }
        CharacterCallError::Handler(handler) => handler_class(handler, provider, redactor),
        // the provider name is the caller's input and the call can never succeed, so LiteLLM's ValueError becomes a 400
        CharacterCallError::UnknownProvider { .. }
        | CharacterCallError::CreateUnsupported { .. }
        | CharacterCallError::GetUnsupported { .. } => ErrorClass::BadRequest(Box::new(
            ResponseDetails::for_provider(error.to_string(), Some(MODEL.to_owned()), provider),
        )),
        CharacterCallError::Mock(_)
        | CharacterCallError::Selection(_)
        | CharacterCallError::Timeout { .. }
        | CharacterCallError::Logging(_) => value_error(error.to_string(), provider, redactor),
    };
    CharacterCallFailure { error, class }
}

// a provider status keeps the class exception_type gives it, and every other handler failure is the catchall
fn handler_class(
    error: &CharacterHandlerFailure,
    provider: &str,
    redactor: &Redactor,
) -> ErrorClass {
    error.status_code.map_or_else(
        || value_error(error.to_string(), provider, redactor),
        |status_code| {
            dispatch(
                &DispatchInput::Raw(Box::new(exception(
                    error.to_string(),
                    provider,
                    Some(status_code),
                ))),
                redactor,
            )
        },
    )
}

fn value_error(message: String, provider: &str, redactor: &Redactor) -> ErrorClass {
    let mut raw = exception(message, provider, None);
    "ValueError".clone_into(&mut raw.exception_type);
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// docs/compatibility.md rejects a value outside its declared type with a 400 that names the field
fn named_field_class(message: String, field: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message,
        model: Some(MODEL.to_owned()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

fn exception(message: String, provider: &str, status_code: Option<u16>) -> RawException {
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
