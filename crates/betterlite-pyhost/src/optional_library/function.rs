use pyo3::exceptions::PyAttributeError;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Bound, Py, PyAny, PyErr, Python};

use crate::call_error::PythonError;

use super::LibraryError;

pub(crate) fn resolve(
    py: Python<'_>,
    module: &Bound<'_, PyAny>,
    module_name: &str,
    path: &str,
) -> Result<Py<PyAny>, LibraryError> {
    let mut object = module.clone();
    for name in path.split('.') {
        object = object
            .getattr(name)
            .map_err(|error| attribute_error(py, module_name, path, &error))?;
    }
    if !object.is_callable() {
        return Err(LibraryError::NotCallable {
            module: module_name.to_owned(),
            path: path.to_owned(),
        });
    }
    Ok(object.unbind())
}

fn attribute_error(py: Python<'_>, module: &str, path: &str, error: &PyErr) -> LibraryError {
    let source = Box::new(PythonError::from_pyerr(py, error));
    if error.is_instance_of::<PyAttributeError>(py) {
        return LibraryError::MissingAttribute {
            module: module.to_owned(),
            path: path.to_owned(),
            source,
        };
    }
    LibraryError::Attribute {
        module: module.to_owned(),
        path: path.to_owned(),
        source,
    }
}
