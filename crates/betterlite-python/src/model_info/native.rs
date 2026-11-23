use super::convert::{self, ModelQuery, ProviderQuery, ReloadQuery, RuntimeEntries};
use super::state::Runtime;
use super::{lookup, reload};
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;
use std::sync::Arc;
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeCatalog {
    runtime: Arc<Runtime>,
}
#[pymethods]
impl NativeCatalog {
    #[new]
    pub(crate) fn new(text: &str) -> PyResult<Self> {
        let runtime = Runtime::new(text)?;
        Ok(Self {
            runtime: Arc::new(runtime),
        })
    }
    fn model_info(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let query: ModelQuery = convert::parse(request)?;
        py.detach(|| {
            pyo3_async_runtimes::tokio::get_runtime()
                .block_on(lookup::model_info(&self.runtime, &query))
        })
        .map_err(PyErr::from)
    }
    fn max_tokens(&self, py: Python<'_>, model: &str) -> PyResult<Option<i64>> {
        let model = model.to_owned();
        py.detach(|| {
            pyo3_async_runtimes::tokio::get_runtime()
                .block_on(lookup::max_tokens(&self.runtime, &model))
        })
        .map_err(PyErr::from)
    }
    // ast-grep-ignore: no-padded-one-line-functions - a pymethod is a boundary Python calls by name
    fn provider_info(&self, request: &str) -> PyResult<Option<String>> {
        let query: ProviderQuery = convert::parse(request)?;
        lookup::provider_info(&self.runtime, &query).map_err(PyErr::from)
    }
    fn set_runtime_entries(&self, request: &str) -> PyResult<()> {
        let entries: RuntimeEntries = convert::parse(request)?;
        self.runtime.apply_runtime_entries(entries);
        Ok(())
    }
    fn reload_blocking(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let query: ReloadQuery = convert::parse(request)?;
        py.detach(|| {
            pyo3_async_runtimes::tokio::get_runtime()
                .block_on(reload::reload_blocking(&self.runtime, &query))
        })
        .map_err(PyErr::from)
    }
    fn reload_async<'py>(&self, py: Python<'py>, request: &str) -> PyResult<Bound<'py, PyAny>> {
        let query: ReloadQuery = convert::parse(request)?;
        let runtime = Arc::clone(&self.runtime);
        future_into_py(py, async move {
            reload::reload_async(&runtime, &query)
                .await
                .map_err(PyErr::from)
        })
    }
    // ast-grep-ignore: no-one-line-functions - a pymethod is a boundary Python calls by name
    fn adopt(&self, request: &str) -> PyResult<usize> {
        self.runtime.adopt(request).map_err(PyErr::from)
    }
    // ast-grep-ignore: no-one-line-functions - a pymethod is a boundary Python calls by name
    fn source_info(&self) -> PyResult<String> {
        self.runtime.source_json().map_err(PyErr::from)
    }
    // ast-grep-ignore: no-one-line-functions - a pymethod is a boundary Python calls by name
    fn generation(&self) -> u64 {
        self.runtime.state().generation
    }
}
