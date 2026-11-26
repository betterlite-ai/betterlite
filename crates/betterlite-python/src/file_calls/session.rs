use std::sync::Arc;

use betterlite_core::api::file_http_upload::service::Mode;
use betterlite_core::api::file_provider_read::ReadEntry;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use self::core::FileCallCore;
use super::convert::{parse_create, parse_delete, parse_list, parse_retrieve};

mod core;
mod tests;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeFileCallSession {
    core: Arc<FileCallCore>,
}

#[pymethods]
impl NativeFileCallSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = FileCallCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    fn create(&self, py: Python<'_>, payload: &str, blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let mode = Mode::Sync;
        let mut blobs = blobs;
        let input = parse_create(payload, &mut blobs, mode)?;
        let core = Arc::clone(&self.core);
        py.detach(move || get_runtime().block_on(core.create(&input, mode)))
    }

    fn acreate<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mode = Mode::Asynchronous;
        let mut blobs = blobs;
        let input = parse_create(payload, &mut blobs, mode)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move { core.create(&input, mode).await })
    }

    fn delete(&self, py: Python<'_>, payload: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let input = parse_delete(payload)?;
        let core = Arc::clone(&self.core);
        py.detach(move || get_runtime().block_on(core.delete(&input, Mode::Sync)))
    }

    fn adelete<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mode = Mode::Asynchronous;
        let input = parse_delete(payload)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move { core.delete(&input, mode).await })
    }

    fn retrieve(&self, py: Python<'_>, payload: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let call = parse_retrieve(payload)?;
        let core = Arc::clone(&self.core);
        py.detach(move || get_runtime().block_on(core.retrieve(&call, ReadEntry::Sync)))
    }

    fn aretrieve<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = parse_retrieve(payload)?;
        let core = Arc::clone(&self.core);
        future_into_py(
            py,
            async move { core.retrieve(&call, ReadEntry::Async).await },
        )
    }

    fn list(&self, py: Python<'_>, payload: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let call = parse_list(payload)?;
        let core = Arc::clone(&self.core);
        py.detach(move || get_runtime().block_on(core.list(&call, ReadEntry::Sync)))
    }

    fn alist<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = parse_list(payload)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move { core.list(&call, ReadEntry::Async).await })
    }
}
