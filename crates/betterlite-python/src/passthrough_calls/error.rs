use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};

use super::convert::{ConvertError, ErrorPayload};
use super::session::SessionError;

create_exception!(betterlite_native, NativePassthroughError, PyException);

// every failure the family's Python layer rebuilds: the payload names LiteLLM's exception class
pub(crate) fn native_error(payload: ErrorPayload) -> PyErr {
    match serde_json::to_string(&payload) {
        Ok(text) => NativePassthroughError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
    }
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        match &error {
            SessionError::Json(_) => PyValueError::new_err(error.to_string()),
            SessionError::Proxy { .. } | SessionError::Client { .. } => {
                PyRuntimeError::new_err(error.to_string())
            }
        }
    }
}
