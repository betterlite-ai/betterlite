mod convert;
mod core;
mod error;
mod sandbox;
mod tests;
mod transport;

use std::sync::Arc;

use betterlite_openai::decisions::providers::supported_decisions_providers as provider_names;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyType};
use pyo3_async_runtimes::tokio::future_into_py;

use self::convert::{CallWire, parse};
use self::core::SessionCore;

// One Python object owns the decisions, SystemOne and sandbox calls, and the sandbox tool registry.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeDecisionSession {
    core: Arc<SessionCore>,
    error_type: Py<PyType>,
}

#[pymethods]
impl NativeDecisionSession {
    #[new]
    fn new(py: Python<'_>, settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings)?;
        let error_type = py
            .import("betterlite._native")?
            .getattr("NativeDecisionError")?
            .cast_into::<PyType>()?
            .unbind();
        Ok(Self {
            core: Arc::new(core),
            error_type,
        })
    }

    // the request text names its operation; the blocking form runs on this thread with the GIL released
    fn call(&self, py: Python<'_>, request: &str, _blobs: &Bound<'_, PyAny>) -> PyResult<String> {
        let wire: CallWire = parse(request)?;
        let answer = py.detach(|| self.core.call(&wire));
        answer.map_err(|failure| failure.into_pyerr(py, &self.error_type))
    }

    fn acall<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request)?;
        let core = Arc::clone(&self.core);
        let error_type = self.error_type.clone_ref(py);
        future_into_py(py, async move {
            let answer = core.acall(&wire).await;
            Python::attach(|attached| {
                answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
            })
        })
    }
}

#[pyfunction]
// ast-grep-ignore: no-one-line-functions - the Python seam re-exports the provider list Rust owns
fn supported_decisions_providers() -> Vec<&'static str> {
    provider_names()
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let base = module.getattr("NativeError")?;
    let base_type = base.cast::<PyType>()?;
    let error_type = PyErr::new_type(
        module.py(),
        c"betterlite._native.NativeDecisionError",
        None,
        Some(base_type),
        None,
    )?;
    module.add("NativeDecisionError", error_type)?;
    module.add_class::<NativeDecisionSession>()?;
    module.add_function(wrap_pyfunction!(supported_decisions_providers, module)?)?;
    Ok(())
}
