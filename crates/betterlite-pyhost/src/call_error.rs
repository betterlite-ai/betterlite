use std::time::Duration;

use pyo3::exceptions::PyBaseException;
use pyo3::types::{PyAnyMethods as _, PyStringMethods as _, PyType, PyTypeMethods as _};
use pyo3::{Bound, Py, PyErr, Python};

use crate::blocking_work::WorkError;
use crate::host::PythonHost;
use crate::json_values::ValueError;

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("the Python callable did not finish within {after:?}")]
    TimedOut { after: Duration },
    #[error(transparent)]
    Python(#[from] PythonError),
    #[error(transparent)]
    Value(#[from] ValueError),
    #[error(transparent)]
    Work(#[from] WorkError),
}

#[derive(Debug, thiserror::Error)]
#[error("{module}.{type_name}: {message}")]
pub struct PythonError {
    pub type_name: String,
    pub module: String,
    pub message: String,
    pub traceback: String,
    pub raised: Py<PyBaseException>,
}

impl PythonError {
    #[must_use]
    pub fn from_pyerr(py: Python<'_>, error: &PyErr) -> Self {
        let raised = error.value(py);
        let raised_type = error.get_type(py);
        let type_name = raised_type
            .name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let module = raised_type
            .module()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let message = raised
            .str()
            .map(|text| text.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            type_name,
            module,
            message,
            traceback: format_traceback(py, error, &raised_type, raised),
            raised: raised.clone().unbind(),
        }
    }

    #[must_use]
    pub fn is_instance_of(&self, host: &PythonHost, class_path: &str) -> bool {
        let Some((module_name, class_name)) = class_path.rsplit_once('.') else {
            return false;
        };
        host.attach(|py| {
            let Ok(module) = py.import(module_name) else {
                return false;
            };
            let Ok(class) = module.getattr(class_name) else {
                return false;
            };
            self.raised.bind(py).is_instance(&class).unwrap_or(false)
        })
    }
}

fn format_traceback(
    py: Python<'_>,
    error: &PyErr,
    raised_type: &Bound<'_, PyType>,
    raised: &Bound<'_, PyBaseException>,
) -> String {
    let Ok(module) = py.import("traceback") else {
        return String::new();
    };
    let traceback = error
        .traceback(py)
        .map_or_else(|| py.None(), |traceback| traceback.into_any().unbind());
    let Ok(lines) = module.call_method1("format_exception", (raised_type, raised, traceback))
    else {
        return String::new();
    };
    let Ok(formatted) = lines.extract::<Vec<String>>() else {
        return String::new();
    };
    formatted.concat()
}

mod tests;
