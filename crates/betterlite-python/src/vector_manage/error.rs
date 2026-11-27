use std::ffi::CStr;

use betterlite_config::api::vector_store_read::SettingsError;
use betterlite_config::loader::ConfigError;
use betterlite_core::api::vector_store_read::error::VectorStoreReadFailure;
use betterlite_core::api::vector_store_write::error::VectorStoreWriteFailure;
use betterlite_env::EnvError;
use betterlite_redact::error::RedactionError;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;

use super::convert::{ConvertError, FieldError, NativeErrorPayload, read_payload, write_payload};

const CLASS_NAME: &str = "NativeVectorManageError";
// CPython's PyErr_NewException takes a <module>.<ExceptionName> name; a bare one aborts _native's init
const CLASS_NAME_C: &CStr = c"betterlite._native.NativeVectorManageError";

// The Python exception a vector store failure is raised as, created once as a subclass of the family base.
static CLASS: PyOnceLock<Py<PyType>> = PyOnceLock::new();

// The first registration decides the base; a later one reuses the class its own module already holds
fn class_with<'py>(py: Python<'py>, base: &Bound<'py, PyType>) -> PyResult<Py<PyType>> {
    let class = CLASS.get_or_try_init(py, || {
        PyErr::new_type(py, CLASS_NAME_C, None, Some(base), None)
    })?;
    Ok(class.clone_ref(py))
}

// LiteLLM's exception class for this family: the completion session registers NativeError before this family runs
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let base = match module.getattr("NativeError") {
        Ok(found) => found.cast_into::<PyType>()?,
        Err(_missing) => module.py().get_type::<PyException>(),
    };
    let class = class_with(module.py(), &base)?;
    module.add(CLASS_NAME, class)
}

// What a caller sees when a call fails: the SDK exception class, or the local refusal of a value
pub(crate) fn raise(py: Python<'_>, failure: &NativeFailure) -> PyErr {
    match failure {
        NativeFailure::Field(error) => PyValueError::new_err(error.to_string()),
        NativeFailure::Setup { message } => PyRuntimeError::new_err(message.clone()),
        NativeFailure::Read { failure, provider } => exception(py, read_payload(failure, provider)),
        NativeFailure::Write { failure, provider } => {
            exception(py, write_payload(failure, provider))
        }
    }
}

// The exception instance carries the JSON the family's Python error class reads back into its attributes
fn exception(py: Python<'_>, payload: NativeErrorPayload) -> PyErr {
    match serde_json::to_string(&payload) {
        Ok(text) => match class_with(py, &py.get_type::<PyException>()) {
            Ok(class) => PyErr::from_type(class.bind(py).clone(), text),
            Err(unavailable) => unavailable,
        },
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
    }
}

// Everything one native call can fail with: the read core's failure, the write core's or a value refused on entry
#[derive(Debug, thiserror::Error)]
pub(crate) enum NativeFailure {
    #[error(transparent)]
    Field(#[from] FieldError),
    #[error("the vector store call could not be logged: {message}")]
    Setup { message: String },
    #[error("{failure}")]
    Read {
        failure: VectorStoreReadFailure,
        provider: String,
    },
    #[error("{failure}")]
    Write {
        failure: VectorStoreWriteFailure,
        provider: String,
    },
}

// Everything that can fail while a session is built, before any request is made
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error("the configuration the session reads could not be loaded: {source}")]
    Config {
        #[source]
        source: ConfigError,
    },
    #[error("the request timeout setting could not be read: {source}")]
    Settings {
        #[source]
        source: SettingsError,
    },
    #[error("the write timeout setting could not be read: {source}")]
    Timeout {
        #[source]
        source: ConfigError,
    },
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the credential redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        let text = error.to_string();
        match &error {
            ConvertError::Json { .. } | ConvertError::Files { .. } => PyValueError::new_err(text),
            ConvertError::Encode { .. } => PyRuntimeError::new_err(text),
        }
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        match &error {
            SessionError::Json(ConvertError::Json { .. } | ConvertError::Files { .. }) => {
                PyValueError::new_err(error.to_string())
            }
            SessionError::Json(ConvertError::Encode { .. })
            | SessionError::Config { .. }
            | SessionError::Settings { .. }
            | SessionError::Timeout { .. }
            | SessionError::Proxy { .. }
            | SessionError::Redactor { .. } => PyRuntimeError::new_err(error.to_string()),
        }
    }
}
