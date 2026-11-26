use super::convert::ConvertError;
pub(crate) const MISSING_KEY_MESSAGE: &str = "The api_key client option must be set by passing api_key to the client or by setting the OPENAI_API_KEY environment variable";
use crate::completion::error::SessionError as CompletionSessionError;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::metadata::RetryMetadata;
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use serde::Serialize;
use std::collections::BTreeMap;

create_exception!(betterlite_native, NativeAudioError, PyException);

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
}

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
    pub(crate) fn plain(kind: &'static str, message: String, model: Option<String>) -> Self {
        Self {
            kind,
            message,
            model: model.map(String::into_boxed_str),
            provider: None,
            status_code: None,
            headers: BTreeMap::new(),
            retry: RetryMetadata::default(),
        }
    }

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
        Ok(text) => NativeAudioError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(failure.message.clone()),
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        let message = error.to_string();
        match error {
            SessionError::Json(ConvertError::Json { .. }) => PyValueError::new_err(message),
            SessionError::Json(_) | SessionError::Completion(_) | SessionError::Redact { .. } => {
                PyRuntimeError::new_err(message)
            }
        }
    }
}

pub(crate) const fn context(
    message: String,
    model: Option<String>,
    provider: Option<String>,
) -> ErrorContext {
    ErrorContext {
        message,
        model,
        provider,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

// A conversion the provider never saw: LiteLLM raises a ValueError for these.
impl From<ConvertError> for CallFailure {
    fn from(error: ConvertError) -> Self {
        Self::plain("ValueError", error.to_string(), None)
    }
}

pub(crate) fn missing_key(model: &str) -> CallFailure {
    CallFailure::plain(
        "APIConnectionError",
        String::from(MISSING_KEY_MESSAGE),
        Some(model.to_owned()),
    )
}

pub(crate) fn provider_failure(
    message: String,
    status: Option<u16>,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallFailure {
    let raw = RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message,
        status_code: status,
        status_code_is_synthesized: false,
        message: None,
        body: Some(RawExceptionBody::default()),
        exception_type: String::new(),
    };
    let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
    CallFailure::from_class(&class)
}
