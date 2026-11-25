use super::convert::ConvertError;
use crate::completion::error::SessionError as CompletionSessionError;
use crate::embeddings::convert::NativeEmbeddingErrorPayload;
use betterlite_context::media_optional_params::error::MediaParamsError;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::error::class_name;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use serde::Serialize;

create_exception!(betterlite_native, NativeImageError, PyException);

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
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) model: Option<String>,
    #[serde(rename = "llm_provider")]
    pub(crate) provider: Option<String>,
    #[serde(skip)]
    pub(crate) class: Option<Box<ErrorClass>>,
}

impl CallFailure {
    pub(crate) fn plain(kind: &str, message: String, model: Option<String>) -> Self {
        Self {
            kind: kind.to_owned(),
            message,
            model,
            provider: None,
            class: None,
        }
    }

    pub(crate) fn from_class(class: ErrorClass, message: String) -> Self {
        let metadata = class.metadata();
        Self {
            kind: class_name(&class).to_owned(),
            message,
            model: metadata.model,
            provider: metadata.provider,
            class: Some(Box::new(class)),
        }
    }
}

pub(crate) fn class_error(failure: &CallFailure) -> PyErr {
    let encoded = failure.class.as_ref().map_or_else(
        || serde_json::to_string(failure),
        |class| serde_json::to_string(&NativeEmbeddingErrorPayload::from_class(class)),
    );
    match encoded {
        Ok(text) => NativeImageError::new_err(text),
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
        error_str: message.clone(),
        status_code: status,
        status_code_is_synthesized: false,
        message: None,
        body: Some(RawExceptionBody::default()),
        exception_type: String::new(),
    };
    let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
    CallFailure::from_class(class, message)
}

// LiteLLM's local ValueErrors become connection errors, except the edit parameter check's own class
pub(crate) fn convert_failure(
    error: &ConvertError,
    model: Option<String>,
    provider: Option<String>,
) -> CallFailure {
    let message = error.to_string();
    let details = Box::new(context(message.clone(), model, provider));
    let class = if matches!(
        error,
        ConvertError::UnsupportedParams { .. }
            | ConvertError::Media(MediaParamsError::ImageUnsupported { .. })
    ) {
        ErrorClass::UnsupportedParams(details)
    } else {
        ErrorClass::ApiConnection(details)
    };
    CallFailure::from_class(class, message)
}

mod tests;
