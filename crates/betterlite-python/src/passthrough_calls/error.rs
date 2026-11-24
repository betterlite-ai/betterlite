use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};

use super::convert::{ConvertError, ErrorPayload};
use super::helpers::HelperError;
use super::session::SessionError;

create_exception!(betterlite_native, NativePassthroughError, PyException);

// every failure the family's Python layer rebuilds: the payload names LiteLLM's exception class
pub(crate) fn native_error(payload: ErrorPayload) -> PyErr {
    match serde_json::to_string(&payload) {
        Ok(text) => NativePassthroughError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
    }
}

fn convert_error(error: &ConvertError) -> PyErr {
    match error {
        ConvertError::Encode { .. } => PyRuntimeError::new_err(error.to_string()),
        ConvertError::Json { .. } | ConvertError::File { .. } | ConvertError::Content => {
            PyValueError::new_err(error.to_string())
        }
    }
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        convert_error(&error)
    }
}

impl From<HelperError> for PyErr {
    fn from(error: HelperError) -> Self {
        match &error {
            HelperError::Url(_) | HelperError::Timeout(_) => {
                PyValueError::new_err(error.to_string())
            }
            HelperError::Wire(wire) => convert_error(wire),
        }
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        if let SessionError::Json(wire) = error {
            return Self::from(wire);
        }
        PyRuntimeError::new_err(error.to_string())
    }
}
