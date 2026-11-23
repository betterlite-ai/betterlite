use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_config::loader::ConfigError;
use betterlite_core::api::container_create_list::error::ContainerOpFailure as CollectionFailure;
use betterlite_core::api::container_resource_ops::error::ContainerOpFailure as ResourceFailure;
use betterlite_core::cost::completion::lookup::LookupError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::catalog::CatalogError;
use betterlite_redact::error::RedactionError;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::types::PyType;
use pyo3::{PyErr, Python};

use super::convert::{ConvertError, NativeContainerPayload};

// A failure outside the provider call: the session could not be built or its settings read
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
        source: HttpError,
    },
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

// A native method answers the class LiteLLM's exception_type would raise, or a local failure it never saw
#[derive(Debug)]
pub(crate) enum SessionFailure {
    Raised(NativeContainerPayload),
    Setup(SessionError),
}

impl SessionFailure {
    pub(crate) fn into_pyerr(self, py: Python<'_>, error_type: &Py<PyType>) -> PyErr {
        match self {
            Self::Raised(payload) => native_error(py, error_type, &payload),
            Self::Setup(error) => PyErr::from(error),
        }
    }
}

// The error text is the JSON payload raise_native_error parses, so the Python class is LiteLLM's
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
        Self::Raised(NativeContainerPayload::from_class(&failure.class))
    }
}

impl From<ResourceFailure> for SessionFailure {
    fn from(failure: ResourceFailure) -> Self {
        Self::Raised(NativeContainerPayload::from_class(&failure.class))
    }
}

impl From<SessionError> for SessionFailure {
    fn from(error: SessionError) -> Self {
        Self::Setup(error)
    }
}

impl From<ConvertError> for SessionFailure {
    fn from(error: ConvertError) -> Self {
        Self::Setup(SessionError::from(error))
    }
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        match &error {
            ConvertError::Json { .. } => PyValueError::new_err(error.to_string()),
            ConvertError::Encode { .. }
            | ConvertError::Timeout { .. }
            | ConvertError::QueryValue { .. } => PyRuntimeError::new_err(error.to_string()),
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
