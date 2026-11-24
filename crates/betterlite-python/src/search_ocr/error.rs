use betterlite_config::tls::TlsPolicyError;
use betterlite_context::core_settings::SettingsError;
use betterlite_env::EnvError;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::pool::PoolError;
use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;

use crate::completion::error::NativeError;

use super::convert::ConvertError;

mod tests;

// Everything the session needs before it can answer a call; each variant names the outside value that failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[source]
        source: PoolError,
    },
    #[error("the TLS policy could not be resolved: {source}")]
    Tls {
        #[source]
        source: TlsPolicyError,
    },
    #[error("the request timeout could not be read: {source}")]
    Settings {
        #[source]
        source: SettingsError,
    },
    #[error("the redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
}

// A finished call fails either before it starts or as the class LiteLLM raises for the provider's answer.
#[derive(Debug)]
pub(crate) enum NativeFailure {
    Convert(ConvertError),
    Session(SessionError),
    Class {
        class: ErrorClass,
        message: String,
        headers: Vec<(String, String)>,
    },
}

impl NativeFailure {
    #[must_use]
    pub(crate) fn into_pyerr(self) -> PyErr {
        match self {
            Self::Convert(error) => PyErr::from(error),
            Self::Session(error) => PyErr::from(error),
            Self::Class {
                class,
                message,
                headers,
            } => native_error(&class, &message, &headers),
        }
    }
}

impl From<ConvertError> for NativeFailure {
    fn from(error: ConvertError) -> Self {
        Self::Convert(error)
    }
}

// The payload NativeError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Serialize)]
struct NativeErrorPayload {
    #[serde(rename = "type")]
    kind: String,
    message: String,
    status_code: Option<u16>,
    llm_provider: Option<String>,
    model: Option<String>,
    body: Option<Json>,
    headers: BTreeMap<String, String>,
    num_retries: Option<i64>,
    max_retries: Option<i64>,
}

// The payload NativeError carries: LiteLLM's exception name, the answer's message and the class's own fields.
fn error_payload(
    class: &ErrorClass,
    message: &str,
    headers: &[(String, String)],
) -> NativeErrorPayload {
    let metadata = class.metadata();
    let mut mapped_headers = BTreeMap::new();
    if let ErrorClass::RateLimit(details) = class {
        mapped_headers.clone_from(&details.headers);
    }
    for (name, value) in headers {
        mapped_headers.insert(name.clone(), value.clone());
    }
    NativeErrorPayload {
        kind: class_name(class).to_owned(),
        message: message.to_owned(),
        status_code: class_status(class),
        llm_provider: metadata.provider,
        model: metadata.model,
        body: None,
        headers: mapped_headers,
        num_retries: None,
        max_retries: metadata.retry.max_retries.map(i64::from),
    }
}

// The family's one exception: NativeError under its own name, carrying the payload raise_native_error reads.
pub(crate) fn native_error(
    class: &ErrorClass,
    message: &str,
    headers: &[(String, String)],
) -> PyErr {
    let payload = error_payload(class, message, headers);
    match serde_json::to_string(&payload) {
        Ok(text) => NativeError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        if !matches!(&error, SessionError::Json(_)) {
            return PyRuntimeError::new_err(error.to_string());
        }
        PyValueError::new_err(error.to_string())
    }
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        match &error {
            ConvertError::Encode { .. } => PyRuntimeError::new_err(error.to_string()),
            ConvertError::Json { .. } | ConvertError::Timeout { .. } => {
                PyValueError::new_err(error.to_string())
            }
        }
    }
}
