use std::collections::BTreeMap;
use std::time::TryFromFloatSecsError;

use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_config::tls::TlsPolicyError;
use betterlite_core::api::video_character_calls::error::CharacterCallFailure;
use betterlite_core::api::video_edit_call::error::VideoEditFailure;
use betterlite_core::api::video_extension_call::error::VideoExtensionFailure;
use betterlite_error::error::metadata::RetryMetadata;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::error::RedactionError;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use serde::Serialize;

use super::convert::ConvertError;
use crate::completion::error::SessionError as CompletionSessionError;

create_exception!(betterlite_native, NativeVideoEditError, PyException);

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error(transparent)]
    Completion(#[from] CompletionSessionError),
    #[error(transparent)]
    Tls(#[from] TlsPolicyError),
    #[error("the redactor could not be built: {source}")]
    Redact {
        #[from]
        source: RedactionError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error("the request timeout is not usable: {0}")]
    Timeout(#[from] TimeoutError),
    #[error("REQUEST_TIMEOUT is not a usable number of seconds: {0}")]
    RequestTimeout(#[source] TryFromFloatSecsError),
}

// A provider fault as B3335 reports it: the status it came with, and the text to map
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct CharacterProviderError {
    pub(crate) status_code: Option<u16>,
    pub(crate) message: String,
}

// one call's failure: the class LiteLLM would raise, written as the payload raise_native_error reads
#[derive(Debug, Serialize)]
pub(crate) struct CallFailure {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) message: String,
    pub(crate) model: Option<Box<str>>,
    #[serde(rename = "llm_provider")]
    pub(crate) provider: Option<Box<str>>,
    pub(crate) status_code: Option<u16>,
    headers: BTreeMap<String, String>,
    #[serde(flatten)]
    retry: RetryMetadata,
}

impl CallFailure {
    pub(crate) fn from_class(class: &ErrorClass) -> Self {
        let metadata = class.metadata();
        let headers = if let ErrorClass::RateLimit(details) = class {
            details.headers.clone()
        } else {
            BTreeMap::new()
        };
        Self {
            kind: class_name(class),
            message: class_message(class),
            model: metadata.model.map(String::into_boxed_str),
            provider: metadata.provider.map(String::into_boxed_str),
            status_code: class_status(class),
            headers,
            retry: metadata.retry,
        }
    }
}

pub(crate) fn class_error(failure: &CallFailure) -> PyErr {
    match serde_json::to_string(failure) {
        Ok(text) => NativeVideoEditError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(failure.message.clone()),
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        let message = error.to_string();
        match error {
            SessionError::Json(ConvertError::Json { .. }) => PyValueError::new_err(message),
            SessionError::Json(_)
            | SessionError::Completion(_)
            | SessionError::Tls(_)
            | SessionError::Redact { .. }
            | SessionError::Logging(_)
            | SessionError::Timeout(_)
            | SessionError::RequestTimeout(_) => PyRuntimeError::new_err(message),
        }
    }
}

// One call's answer: a session problem is a Python error, a call failure the class LiteLLM raises
#[derive(Debug)]
pub(crate) enum RunError {
    Session(SessionError),
    Call(CallFailure),
}

impl From<SessionError> for RunError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl From<CallFailure> for RunError {
    fn from(failure: CallFailure) -> Self {
        Self::Call(failure)
    }
}

impl From<ConvertError> for RunError {
    fn from(error: ConvertError) -> Self {
        Self::Session(SessionError::from(error))
    }
}

impl From<CharacterCallFailure> for RunError {
    fn from(failure: CharacterCallFailure) -> Self {
        Self::Call(CallFailure::from_class(&failure.class))
    }
}

impl From<VideoEditFailure> for RunError {
    fn from(failure: VideoEditFailure) -> Self {
        Self::Call(CallFailure::from_class(&failure.class))
    }
}

impl From<VideoExtensionFailure> for RunError {
    fn from(failure: VideoExtensionFailure) -> Self {
        Self::Call(CallFailure::from_class(&failure.class))
    }
}

impl From<RunError> for PyErr {
    fn from(error: RunError) -> Self {
        match error {
            RunError::Session(source) => Self::from(source),
            RunError::Call(failure) => class_error(&failure),
        }
    }
}
