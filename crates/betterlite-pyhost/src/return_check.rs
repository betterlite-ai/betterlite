use pyo3::exceptions::PyTypeError;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Py, PyAny, PyErr, Python};

use crate::call_error::PythonError;
use crate::callable_handle::CallableHandle;
use crate::host::PythonHost;

#[derive(Debug, thiserror::Error)]
pub enum ReturnCheckError {
    #[error(transparent)]
    Python(#[from] PythonError),
}

pub fn check_literal_true_return(
    host: &PythonHost,
    handle: Option<&CallableHandle>,
) -> Result<Option<Py<PyAny>>, ReturnCheckError> {
    let Some(checked) = handle else {
        return Ok(None);
    };
    host.attach(|py| check_in_interpreter(py, checked))
        .map(Some)
}

fn check_in_interpreter(
    py: Python<'_>,
    handle: &CallableHandle,
) -> Result<Py<PyAny>, ReturnCheckError> {
    let failure = |error: PyErr| ReturnCheckError::Python(PythonError::from_pyerr(py, &error));
    let typing = py.import("typing").map_err(failure)?;
    let hints = typing
        .getattr("get_type_hints")
        .and_then(|get_type_hints| get_type_hints.call1((handle.object.bind(py),)))
        .map_err(failure)?;
    let return_type = hints.call_method1("get", ("return",)).map_err(failure)?;
    let literal = typing
        .getattr("Literal")
        .and_then(|literal| literal.get_item(true))
        .map_err(failure)?;
    if return_type.eq(&literal).map_err(failure)? {
        return Ok(handle.object.clone_ref(py));
    }
    let rendered = return_type
        .repr()
        .and_then(|text| text.extract::<String>())
        .map_err(failure)?;
    let error = PyTypeError::new_err(format!(
        "Custom validator must be annotated to return Literal[True], got {rendered}"
    ));
    Err(ReturnCheckError::Python(PythonError::from_pyerr(
        py, &error,
    )))
}

mod tests;
