use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyType;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProviderError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("{message}")]
    BadRequest {
        message: String,
        model: Option<String>,
    },
}

// Python's raise_native_error reads the class name from `type` and the text from `message` for the shared NativeError.
#[derive(Serialize)]
struct BadRequestPayload<'message> {
    #[serde(rename = "type")]
    kind: &'static str,
    message: &'message str,
    llm_provider: &'static str,
    model: Option<&'message str>,
}

impl ProviderError {
    pub(crate) fn into_pyerr(self, py: Python<'_>) -> PyErr {
        match self {
            Self::Json { source } => PyValueError::new_err(format!(
                "the native request text is not JSON of the expected shape: {source}"
            )),
            Self::Encode { source } => PyRuntimeError::new_err(format!(
                "the answer could not be written as JSON: {source}"
            )),
            Self::BadRequest { message, model } => {
                native_bad_request(py, &message, model.as_deref())
            }
        }
    }
}

fn native_bad_request(py: Python<'_>, message: &str, model: Option<&str>) -> PyErr {
    let payload = BadRequestPayload {
        kind: "BadRequestError",
        message,
        llm_provider: "",
        model,
    };
    let Ok(text) = serde_json::to_string(&payload) else {
        return PyRuntimeError::new_err(message.to_owned());
    };
    match native_error_class(py) {
        Ok(class) => PyErr::from_type(class, text),
        Err(missing) => missing,
    }
}

// The exception class lives in betterlite._native, whose Rust module is private to the completion family.
fn native_error_class(py: Python<'_>) -> PyResult<Bound<'_, PyType>> {
    py.import("betterlite._native")?
        .getattr("NativeError")?
        .cast_into::<PyType>()
        .map_err(PyErr::from)
}
