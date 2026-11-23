use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyType};
use pyo3_async_runtimes::tokio::future_into_py;

use self::core::SessionCore;
use super::convert::{CallWire, parse, query_values, requested_timeout};

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

    // the blocking calls below run on this thread with the GIL released; the async twins hand back a coroutine
    fn create_container(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<String> {
        let wire: CallWire = parse(request)?;
        let call = wire.arguments.create()?;
        let answer = py.detach(|| self.core.create(&wire, &call));
        answer.map_err(|failure| failure.into_pyerr(py, &self.error_type))
    }

    fn acreate_container<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request)?;
        let call = wire.arguments.create()?;
        let core = Arc::clone(&self.core);
        let error_type = self.error_type.clone_ref(py);
        future_into_py(py, async move {
            let answer = core.acreate(&wire, &call).await;
            Python::attach(|attached| {
                answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
            })
        })
    }

    fn list_containers(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<String> {
        let wire: CallWire = parse(request)?;
        let call = wire.arguments.list()?;
        let answer = py.detach(|| self.core.list(&wire, &call));
        answer.map_err(|failure| failure.into_pyerr(py, &self.error_type))
    }

    fn alist_containers<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request)?;
        let call = wire.arguments.list()?;
        let core = Arc::clone(&self.core);
        let error_type = self.error_type.clone_ref(py);
        future_into_py(py, async move {
            let answer = core.alist(&wire, &call).await;
            Python::attach(|attached| {
                answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
            })
        })
    }

    fn retrieve_container(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<String> {
        let wire: CallWire = parse(request)?;
        let extra_query = query_values(wire.arguments.extra_query.as_ref())?;
        let timeout = requested_timeout(wire.arguments.timeout)?;
        let answer = py.detach(|| {
            let call = wire
                .arguments
                .operation_request(extra_query.as_ref(), timeout);
            self.core.retrieve(&wire, &call)
        });
        answer.map_err(|failure| failure.into_pyerr(py, &self.error_type))
    }

    fn aretrieve_container<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request)?;
        let extra_query = query_values(wire.arguments.extra_query.as_ref())?;
        let timeout = requested_timeout(wire.arguments.timeout)?;
        let core = Arc::clone(&self.core);
        let error_type = self.error_type.clone_ref(py);
        future_into_py(py, async move {
            let call = wire
                .arguments
                .operation_request(extra_query.as_ref(), timeout);
            let answer = core.aretrieve(&wire, &call).await;
            Python::attach(|attached| {
                answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
            })
        })
    }

    fn delete_container(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<String> {
        let wire: CallWire = parse(request)?;
        let extra_query = query_values(wire.arguments.extra_query.as_ref())?;
        let timeout = requested_timeout(wire.arguments.timeout)?;
        let answer = py.detach(|| {
            let call = wire
                .arguments
                .operation_request(extra_query.as_ref(), timeout);
            self.core.delete(&wire, &call)
        });
        answer.map_err(|failure| failure.into_pyerr(py, &self.error_type))
    }

    fn adelete_container<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request)?;
        let extra_query = query_values(wire.arguments.extra_query.as_ref())?;
        let timeout = requested_timeout(wire.arguments.timeout)?;
        let core = Arc::clone(&self.core);
        let error_type = self.error_type.clone_ref(py);
        future_into_py(py, async move {
            let call = wire
                .arguments
                .operation_request(extra_query.as_ref(), timeout);
            let answer = core.adelete(&wire, &call).await;
            Python::attach(|attached| {
                answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
            })
        })
    }
}
