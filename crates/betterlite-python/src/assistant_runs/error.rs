use crate::completion::error::SessionError;
use betterlite_config::loader::ConfigError;
use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError};
use serde::Serialize;

create_exception!(betterlite_native, NativeAssistantRunError, PyException);

#[derive(Debug, thiserror::Error)]
pub(crate) enum CallError {
    #[error("the {field} could not be read as JSON of the expected shape: {source}")]
    Json {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("a value could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Transport(#[from] SessionError),
    #[error("the assistant settings could not be read: {source}")]
    Settings {
        #[source]
        source: ConfigError,
    },
    #[error("the request redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
    #[error("the {operation} call answered with a kind of reply this function does not return")]
    Reply { operation: &'static str },
}

// What _completion/errors.py builds a LiteLLM exception from: its class, its message and its attributes.
#[derive(Debug, Serialize)]
pub(crate) struct NativeErrorPayload {
    #[serde(rename = "type")]
    pub(crate) class: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) llm_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body: Option<Json>,
}

#[must_use]
pub(crate) fn payload(class: &str, message: String) -> NativeErrorPayload {
    NativeErrorPayload {
        class: class.to_owned(),
        message,
        status_code: None,
        llm_provider: None,
        model: None,
        body: None,
    }
}

impl NativeErrorPayload {
    pub(crate) fn into_exception(self) -> PyErr {
        match serde_json::to_string(&self) {
            Ok(text) => NativeAssistantRunError::new_err(text),
            Err(_encoding) => PyRuntimeError::new_err(self.message),
        }
    }
}

impl From<CallError> for PyErr {
    fn from(error: CallError) -> Self {
        payload("Exception", error.to_string()).into_exception()
    }
}
