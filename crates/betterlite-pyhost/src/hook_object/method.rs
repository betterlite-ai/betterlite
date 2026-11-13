use pyo3::types::PyAnyMethods;
use pyo3::{Bound, Py, PyAny};

pub(crate) fn callable_attribute(object: &Bound<'_, PyAny>, name: &str) -> Option<Py<PyAny>> {
    object
        .getattr_opt(name)
        .ok()
        .flatten()
        .filter(PyAnyMethods::is_callable)
        .map(Bound::unbind)
}

mod tests;
