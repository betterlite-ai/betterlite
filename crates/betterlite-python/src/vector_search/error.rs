use std::ffi::CStr;

use betterlite_config::api::vector_store_search::SettingsError;
use betterlite_config::loader::ConfigError;
use betterlite_core::api::vector_store_search::error::VectorStoreSearchError;
use betterlite_env::EnvError;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use pyo3::exceptions::{PyException, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;

use super::convert::{ConvertError, NativeRaise, raise_payload};

const CLASS_NAME: &str = "NativeVectorSearchError";
const CLASS_NAME_C: &CStr = c"NativeVectorSearchError";

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

// What a caller sees when a call fails: the SDK exception class, or the bare TypeError the executor check raises
pub(crate) fn raise(py: Python<'_>, error: &VectorStoreSearchError, redactor: &Redactor) -> PyErr {
    match raise_payload(error, redactor) {
        NativeRaise::TypeError(message) => PyTypeError::new_err(message),
        NativeRaise::Exception(payload) => match serde_json::to_string(&payload) {
            Ok(text) => match class_with(py, &py.get_type::<PyException>()) {
                Ok(class) => PyErr::from_type(class.bind(py).clone(), text),
                Err(unavailable) => unavailable,
            },
            Err(_encoding) => PyRuntimeError::new_err(payload.message),
        },
    }
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
            | SessionError::Proxy { .. }
            | SessionError::Redactor { .. } => PyRuntimeError::new_err(error.to_string()),
        }
    }
}
