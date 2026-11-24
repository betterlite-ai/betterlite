mod convert;
mod core;
mod error;
mod stack;
mod transport;
mod uploads;

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyType};
use pyo3_async_runtimes::tokio::future_into_py;

use self::convert::{CallWire, parse};
use self::core::SessionCore;

// litellm.containers.main's file list and upload plus endpoint_factory.py's three generated calls
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let base = module.getattr("NativeError")?;
    let base_type = base.cast::<PyType>()?;
    let error_type = PyErr::new_type(
        module.py(),
        c"betterlite._native.NativeContainerFileError",
        None,
        Some(base_type),
        None,
    )?;
    module.add("NativeContainerFileError", error_type)?;
    module.add_class::<NativeContainerFileSession>()?;
    Ok(())
}

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeContainerFileSession {
    core: Arc<SessionCore>,
    error_type: Py<PyType>,
}

#[pymethods]
impl NativeContainerFileSession {
    #[new]
    fn new(py: Python<'_>, settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings)?;
        let error_type = py
            .import("betterlite._native")?
            .getattr("NativeContainerFileError")?
            .cast_into::<PyType>()?
            .unbind();
        Ok(Self {
            core: Arc::new(core),
            error_type,
        })
    }

    // list and upload answer JSON; the file bytes the caller sent sit beside the request text
    fn call(&self, py: Python<'_>, request: &str, blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let wire: CallWire = parse(request)?;
        py.detach(|| self.core.call(&wire, &mut blobs))
            .map_err(|failure| failure.into_pyerr(py, &self.error_type))
    }

    fn acall<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request)?;
        let core = Arc::clone(&self.core);
        let error_type = self.error_type.clone_ref(py);
        future_into_py(py, async move {
            let answer = core.acall(&wire, &mut blobs).await;
            Python::attach(|attached| {
                answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
            })
        })
    }

    // retrieve_container_file_content answers the provider's bytes as they came
    fn content(&self, py: Python<'_>, request: &str) -> PyResult<Vec<u8>> {
        let wire: CallWire = parse(request)?;
        py.detach(|| self.core.content(&wire))
            .map_err(|failure| failure.into_pyerr(py, &self.error_type))
    }

    fn acontent<'py>(&self, py: Python<'py>, request: &str) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request)?;
        let core = Arc::clone(&self.core);
        let error_type = self.error_type.clone_ref(py);
        future_into_py(py, async move {
            let answer = core.acontent(&wire).await;
            Python::attach(|attached| {
                answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
            })
        })
    }
}
