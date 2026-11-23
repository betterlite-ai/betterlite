use std::sync::Arc;

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert;
use super::core::CacheCore;

mod tests;

// LiteLLM's Cache over a native store: every call arrives as JSON text and answers JSON text.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeCache {
    core: Arc<CacheCore>,
}

#[pymethods]
impl NativeCache {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let wire = convert::parse_settings(settings)?;
        let core = get_runtime().block_on(CacheCore::open(&wire))?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    fn cache_key(&self, request: &str) -> PyResult<String> {
        let call = convert::parse_call(request)?;
        self.core.cache_key(&call).map_err(PyErr::from)
    }

    fn async_cache_key<'py>(&self, py: Python<'py>, request: &str) -> PyResult<Bound<'py, PyAny>> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(
            py,
            async move { core.cache_key(&call).map_err(PyErr::from) },
        )
    }

    fn lookup(&self, request: &str) -> PyResult<String> {
        let call = convert::parse_call(request)?;
        self.core.lookup(&call).map_err(PyErr::from)
    }

    fn async_lookup<'py>(&self, py: Python<'py>, request: &str) -> PyResult<Bound<'py, PyAny>> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.async_lookup(&call).await;
            outcome.map_err(PyErr::from)
        })
    }

    fn lookup_semantic(&self, request: &str) -> PyResult<String> {
        let call = convert::parse_call(request)?;
        self.core.lookup_semantic(&call).map_err(PyErr::from)
    }

    fn async_lookup_semantic<'py>(
        &self,
        py: Python<'py>,
        request: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.async_lookup_semantic(&call).await;
            outcome.map_err(PyErr::from)
        })
    }

    fn write(&self, request: &str) -> PyResult<()> {
        let call = convert::parse_call(request)?;
        self.core.write(&call).map_err(PyErr::from)
    }

    fn async_write<'py>(&self, py: Python<'py>, request: &str) -> PyResult<Bound<'py, PyAny>> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.async_write(&call).await;
            outcome.map_err(PyErr::from)
        })
    }

    fn write_embedding(&self, request: &str) -> PyResult<String> {
        let call = convert::parse_call(request)?;
        self.core.write_embedding(&call).map_err(PyErr::from)
    }

    fn async_write_embedding<'py>(
        &self,
        py: Python<'py>,
        request: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            core.write_embedding(&call).map_err(PyErr::from)
        })
    }

    fn write_pipeline(&self, py: Python<'_>, request: &str) -> PyResult<()> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        py.detach(|| {
            get_runtime()
                .block_on(core.async_write_pipeline(&call))
                .map_err(PyErr::from)
        })
    }

    fn async_write_pipeline<'py>(
        &self,
        py: Python<'py>,
        request: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.async_write_pipeline(&call).await;
            outcome.map_err(PyErr::from)
        })
    }

    fn batch_write(&self, py: Python<'_>, request: &str) -> PyResult<()> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        py.detach(|| {
            get_runtime()
                .block_on(core.async_batch_write(&call))
                .map_err(PyErr::from)
        })
    }

    fn async_batch_write<'py>(
        &self,
        py: Python<'py>,
        request: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = convert::parse_call(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.async_batch_write(&call).await;
            outcome.map_err(PyErr::from)
        })
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let core = Arc::clone(&self.core);
        py.detach(|| get_runtime().block_on(core.close()).map_err(PyErr::from))
    }

    fn async_close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.close().await;
            outcome.map_err(PyErr::from)
        })
    }
}
