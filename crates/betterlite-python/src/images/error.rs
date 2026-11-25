use super::convert::ConvertError;
use crate::completion::error::SessionError as CompletionSessionError;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use serde::Serialize;
use std::collections::BTreeMap;

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

// What every failure carries: either a mapped class, or LiteLLM's bare ValueError for image_variation.
#[derive(Debug)]
pub(crate) struct CallFailure {
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
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

    fn from_class(
        class: ErrorClass,
        message: String,
        model: Option<String>,
        provider: Option<String>,
    ) -> Self {
        Self {
            kind: class_name(&class).to_owned(),
            message,
            model,
            provider,
            class: Some(Box::new(class)),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct NativeImageErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
}

impl NativeImageErrorPayload {
    fn from_class(class: &ErrorClass) -> Self {
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = class {
            headers.clone_from(&details.headers);
        }
        let metadata = class.metadata();
        Self {
            kind: class_name(class).to_owned(),
            message: class_message(class),
            status_code: class_status(class),
            llm_provider: metadata.provider,
            model: metadata.model,
            headers,
        }
    }
}

pub(crate) fn class_error(failure: &CallFailure) -> PyErr {
    let payload = match &failure.class {
        Some(class) => NativeImageErrorPayload::from_class(class),
        None => NativeImageErrorPayload {
            kind: failure.kind.clone(),
            message: failure.message.clone(),
            status_code: None,
            llm_provider: failure.provider.clone(),
            model: failure.model.clone(),
            headers: BTreeMap::new(),
        },
    };
    match serde_json::to_string(&payload) {
        Ok(text) => NativeImageError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
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

fn context(message: String, model: Option<String>, provider: Option<String>) -> ErrorContext {
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

// a local refusal LiteLLM reaches through ValueError, which its exception mapping turns into a connection error
pub(crate) fn connection_failure(
    message: String,
    model: Option<String>,
    provider: Option<String>,
) -> CallFailure {
    let class = ErrorClass::ApiConnection(Box::new(context(
        message.clone(),
        model.clone(),
        provider.clone(),
    )));
    CallFailure::from_class(class, message, model, provider)
}

// A provider answer is dispatched from its status and message exactly as LiteLLM's exception_type would.
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
    CallFailure::from_class(
        class,
        message,
        Some(model.to_owned()),
        Some(provider.to_owned()),
    )
}

// LiteLLM's local ValueErrors become connection errors, except the edit parameter check's own class
pub(crate) fn convert_failure(
    error: &ConvertError,
    model: Option<String>,
    provider: Option<String>,
) -> CallFailure {
    let message = error.to_string();
    if matches!(
        error,
        ConvertError::UnsupportedParams { .. } | ConvertError::Unsupported(_)
    ) {
        let class = ErrorClass::UnsupportedParams(Box::new(context(
            message.clone(),
            model.clone(),
            provider.clone(),
        )));
        return CallFailure::from_class(class, message, model, provider);
    }
    connection_failure(message, model, provider)
}

mod tests;
