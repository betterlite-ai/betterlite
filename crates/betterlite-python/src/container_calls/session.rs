use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyType};
use pyo3_async_runtimes::tokio::future_into_py;

use self::core::{SessionCore, asynchronous};
use super::convert::{CallWire, parse};

mod core;

// litellm.containers.main's module-level entry points, one session owning the client every call goes through
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeContainerSession {
    core: Arc<SessionCore>,
    error_type: Py<PyType>,
}

#[pymethods]
impl NativeContainerSession {
    #[new]
    fn new(py: Python<'_>, settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings)?;
        let error_type = py
            .import("betterlite._native")?
            .getattr("NativeContainerError")?
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

    // the async twin hands back a coroutine that awaits the same core call
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
            let answer = asynchronous::acall(&core, &wire).await;
            Python::attach(|attached| {
                answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
            })
        })
    }
}
