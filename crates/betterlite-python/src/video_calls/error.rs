use std::collections::BTreeMap;
use std::time::TryFromFloatSecsError;

use super::convert::ConvertError;
use crate::completion::error::SessionError as CompletionSessionError;
use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_core::api::video_content_call::error::VideoContentCallError;
use betterlite_core::api::video_generation_call::error::VideoGenerationFailure;
use betterlite_core::api::video_remix_call::error::VideoCallFailure;
use betterlite_core::api::video_status_call::error::VideoStatusCallError;
use betterlite_error::error::metadata::RetryMetadata;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::error::RedactionError;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use serde::Serialize;

create_exception!(betterlite_native, NativeVideoError, PyException);

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error(transparent)]
    Completion(#[from] CompletionSessionError),
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
    #[error("{0}")]
    Unmapped(String),
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
        Ok(text) => NativeVideoError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(failure.message.clone()),
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        let message = error.to_string();
        match error {
            SessionError::Json(ConvertError::Json { .. }) | SessionError::Unmapped(_) => {
                PyValueError::new_err(message)
            }
            SessionError::Json(_)
            | SessionError::Completion(_)
            | SessionError::Redact { .. }
            | SessionError::Logging(_)
            | SessionError::Timeout(_)
            | SessionError::RequestTimeout(_) => PyRuntimeError::new_err(message),
        }
    }
}

// One video call's answer: a session problem is a Python error, a call failure the class LiteLLM raises
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

impl From<VideoGenerationFailure> for RunError {
    fn from(failure: VideoGenerationFailure) -> Self {
        Self::Call(CallFailure::from_class(&failure.class))
    }
}

impl From<VideoCallFailure> for RunError {
    fn from(failure: VideoCallFailure) -> Self {
        Self::Call(CallFailure::from_class(&failure.class))
    }
}

// The public content entry maps every failure; an unmapped one is a ValueError that never got that far
impl From<VideoContentCallError> for RunError {
    fn from(error: VideoContentCallError) -> Self {
        let VideoContentCallError::Mapped { class, .. } = &error else {
            return Self::Session(SessionError::Unmapped(error.to_string()));
        };
        Self::Call(CallFailure::from_class(class))
    }
}

// The public status entry maps every failure; an unmapped one is a ValueError that never got that far
impl From<VideoStatusCallError> for RunError {
    fn from(error: VideoStatusCallError) -> Self {
        let VideoStatusCallError::Mapped { class, .. } = &error else {
            return Self::Session(SessionError::Unmapped(error.to_string()));
        };
        Self::Call(CallFailure::from_class(class))
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
