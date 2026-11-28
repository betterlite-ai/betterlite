use std::collections::BTreeMap;

use crate::completion::error::SessionError as CompletionSessionError;
use betterlite_config::api::realtime_http_calls::SettingsError;
use betterlite_redact::error::RedactionError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use serde::Serialize;
use thiserror::Error;

create_exception!(betterlite_native, NativeRealtimeError, PyException);

// One failure as main.py's raise_native_error reads it: the class name and the fields that class takes
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CallFailure {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) headers: Option<BTreeMap<String, String>>,
}

impl CallFailure {
    pub(crate) fn class(kind: &str, message: String) -> Self {
        Self {
            kind: kind.to_owned(),
            message,
            status_code: None,
            headers: None,
        }
    }

    pub(crate) fn exception(&self) -> PyErr {
        match serde_json::to_string(self) {
            Ok(payload) => NativeRealtimeError::new_err(payload),
            Err(_error) => NativeRealtimeError::new_err(self.message.clone()),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum SessionError {
    #[error("the realtime session settings are not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the realtime HTTP settings are invalid: {0}")]
    HttpSettings(#[source] SettingsError),
    #[error(transparent)]
    Completion(#[from] CompletionSessionError),
    #[error("the redactor could not be built: {0}")]
    Redact(#[source] RedactionError),
}
