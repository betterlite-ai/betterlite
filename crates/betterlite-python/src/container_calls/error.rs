use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_config::loader::ConfigError;
use betterlite_core::api::container_create_list::error::ContainerOpFailure as CollectionFailure;
use betterlite_core::api::container_resource_ops::error::ContainerOpFailure as ResourceFailure;
use betterlite_core::cost::completion::lookup::LookupError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::catalog::CatalogError;
use betterlite_redact::error::RedactionError;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::types::PyType;
use pyo3::{Py, PyErr, Python};

use super::convert::{ConvertError, NativeContainerPayload};
use crate::completion::error::SessionError as CompletionSessionError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error(transparent)]
    Completion(#[from] CompletionSessionError),
    #[error("the openai credentials could not be read: {source}")]
    Settings {
        #[source]
        source: ConfigError,
    },
    #[error("the request timeout could not be read: {source}")]
    Timeout {
        #[source]
        source: TimeoutError,
    },
    #[error("the bundled model cost map is unavailable: {source}")]
    Catalog {
        #[source]
        source: CatalogError,
    },
    #[error("the cost lookup could not be built: {source}")]
    Lookup {
        #[source]
        source: LookupError,
    },
    #[error("the redactor could not be built: {source}")]
    Redact {
        #[source]
        source: RedactionError,
    },
    #[error("the logging state could not be built: {source}")]
    Logging {
        #[source]
        source: LoggingStateError,
    },
}

#[derive(Debug)]
pub(crate) enum SessionFailure {
    Raised(Box<NativeContainerPayload>),
    Invalid(ConvertError),
    Setup(SessionError),
}

impl SessionFailure {
    pub(crate) fn into_pyerr(self, py: Python<'_>, error_type: &Py<PyType>) -> PyErr {
        match self {
            Self::Raised(payload) => native_error(py, error_type, payload.as_ref()),
            Self::Invalid(error) => PyErr::from(error),
            Self::Setup(error) => PyErr::from(error),
        }
    }
}

pub(crate) fn native_error(
    py: Python<'_>,
    error_type: &Py<PyType>,
    payload: &NativeContainerPayload,
) -> PyErr {
    let message = payload.message.clone();
    match serde_json::to_string(payload) {
        Ok(text) => PyErr::from_type(error_type.bind(py).clone(), text),
        Err(_encoding) => PyRuntimeError::new_err(message),
    }
}

impl From<CollectionFailure> for SessionFailure {
    fn from(failure: CollectionFailure) -> Self {
        Self::Raised(Box::new(NativeContainerPayload::from_class(&failure.class)))
    }
}

impl From<ResourceFailure> for SessionFailure {
    fn from(failure: ResourceFailure) -> Self {
        Self::Raised(Box::new(NativeContainerPayload::from_class(&failure.class)))
    }
}

impl From<SessionError> for SessionFailure {
    fn from(error: SessionError) -> Self {
        Self::Setup(error)
    }
}

impl From<ConvertError> for SessionFailure {
    fn from(error: ConvertError) -> Self {
        Self::Invalid(error)
    }
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        match &error {
            ConvertError::Json { .. } => PyValueError::new_err(error.to_string()),
            ConvertError::Encode { .. }
            | ConvertError::Timeout { .. }
            | ConvertError::QueryValue { .. }
            | ConvertError::Operation { .. } => PyRuntimeError::new_err(error.to_string()),
        }
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        if let SessionError::Json(inner) = &error {
            return PyValueError::new_err(inner.to_string());
        }
        PyRuntimeError::new_err(error.to_string())
    }
}
