use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyType};
use pyo3::{Bound, Py, PyAny, Python};

use crate::call_error::PythonError;
use crate::json_values::{JsonObject, to_object};

use super::{HookError, class_object};

pub(crate) fn build(
    py: Python<'_>,
    class_path: &str,
    entries: &JsonObject,
) -> Result<Py<PyAny>, HookError> {
    let class = class_object(py, class_path)
        .map_err(|error| HookError::Python(PythonError::from_pyerr(py, &error)))?;
    if !class.is_instance_of::<PyType>() {
        return Err(HookError::NotClass {
            path: class_path.to_owned(),
        });
    }
    let keywords = PyDict::new(py);
    for (name, value) in entries {
        let converted = to_object(py, value)
            .map_err(|error| HookError::Python(PythonError::from_pyerr(py, &error)))?;
        keywords
            .set_item(name, converted)
            .map_err(|error| HookError::Python(PythonError::from_pyerr(py, &error)))?;
    }
    class
        .call((), Some(&keywords))
        .map(Bound::unbind)
        .map_err(|error| HookError::Python(PythonError::from_pyerr(py, &error)))
}

mod tests;
