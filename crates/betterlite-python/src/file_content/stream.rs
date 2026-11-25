use std::sync::Arc;

use betterlite_core::api::file_stream_logging::FileContentStreamingResponse;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use tokio::sync::Mutex;

use super::error::stream_failure;

// One streamed file body: the B1080 wrapper's chunks, handed out as the bytes Python iterates.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeFileStream {
    pub(super) stream: Arc<Mutex<FileContentStreamingResponse>>,
    // the entry that produced it: a stream from afile_content is awaitable, one from file_content is not
    #[pyo3(get)]
    pub(super) is_async: bool,
}

#[pymethods]
impl NativeFileStream {
    // the blocking form, waiting on the lock and the next chunk with the GIL released
    pub(crate) fn next_chunk(&self, py: Python<'_>) -> PyResult<Option<Py<PyBytes>>> {
        let stream = Arc::clone(&self.stream);
        let next = py.detach(move || {
            get_runtime().block_on(async move {
                let mut guard = stream.lock().await;
                let mut iter = guard.async_iter()?;
                let chunk = iter.next_chunk().await.transpose();
                drop(iter);
                drop(guard);
                chunk
            })
        });
        let chunk = next.map_err(|error| PyErr::from(stream_failure(&error)))?;
        Ok(chunk.map(|bytes| PyBytes::new(py, &bytes).unbind()))
    }

    // the awaitable form, needing a Python event loop a Rust test has none of
    #[cfg_attr(test, mutants::skip)]
    fn anext_chunk<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let stream = Arc::clone(&self.stream);
        future_into_py(py, async move {
            let mut guard = stream.lock().await;
            let mut iter = guard
                .async_iter()
                .map_err(|error| PyErr::from(stream_failure(&error)))?;
            let chunk = iter
                .next_chunk()
                .await
                .transpose()
                .map_err(|error| PyErr::from(stream_failure(&error)))?;
            drop(iter);
            drop(guard);
            Python::attach(
                |attached| Ok(chunk.map(|bytes| PyBytes::new(attached, &bytes).unbind())),
            )
        })
    }

    // close runs on either stream: an async source's close has no method to await
    pub(crate) fn close(&self, py: Python<'_>) -> PyResult<()> {
        let stream = Arc::clone(&self.stream);
        py.detach(move || {
            get_runtime().block_on(async move {
                let mut guard = stream.lock().await;
                guard.close()
            })
        })
        .map_err(|error| PyErr::from(stream_failure(&error)))
    }

    #[cfg_attr(test, mutants::skip)]
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let stream = Arc::clone(&self.stream);
        future_into_py(py, async move {
            let mut guard = stream.lock().await;
            guard
                .aclose()
                .await
                .map_err(|error| PyErr::from(stream_failure(&error)))
        })
    }
}
