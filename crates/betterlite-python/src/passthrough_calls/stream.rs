use std::sync::Arc;

use betterlite_core::api::passthrough_call::PassthroughRelay;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use tokio::sync::Mutex;

use super::convert::{API_ERROR, ErrorPayload, SYNTHESIZED_STATUS};
use super::error::native_error;

// One relayed upstream body, handed out as the byte chunks Python iterates.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativePassthroughStream {
    relay: Arc<Mutex<PassthroughRelay>>,
}

impl From<Box<PassthroughRelay>> for NativePassthroughStream {
    fn from(relay: Box<PassthroughRelay>) -> Self {
        Self {
            relay: Arc::new(Mutex::new(*relay)),
        }
    }
}

#[pymethods]
impl NativePassthroughStream {
    // the blocking form the synchronous route returns, waiting on the lock with the GIL released
    pub(crate) fn next_chunk(&self, py: Python<'_>) -> PyResult<Option<Py<PyBytes>>> {
        let relay = Arc::clone(&self.relay);
        let next = py.detach(move || {
            get_runtime().block_on(async move {
                let mut guard = relay.lock().await;
                guard.next_chunk().await
            })
        });
        let chunk = next.map_err(|error| {
            native_error(ErrorPayload::failed(
                API_ERROR,
                Some(SYNTHESIZED_STATUS),
                error.to_string(),
            ))
        })?;
        Ok(chunk.map(|bytes| PyBytes::new(py, &bytes).unbind()))
    }

    // the awaitable form, needing a Python event loop a Rust test has none of
    #[cfg_attr(test, mutants::skip)]
    fn anext_chunk<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let relay = Arc::clone(&self.relay);
        future_into_py(py, async move {
            let next = relay.lock().await.next_chunk().await;
            let chunk = next.map_err(|error| {
                native_error(ErrorPayload::failed(
                    API_ERROR,
                    Some(SYNTHESIZED_STATUS),
                    error.to_string(),
                ))
            })?;
            Python::attach(
                |attached| Ok(chunk.map(|bytes| PyBytes::new(attached, &bytes).unbind())),
            )
        })
    }

    pub(crate) fn close(&self, py: Python<'_>) {
        let relay = Arc::clone(&self.relay);
        py.detach(move || {
            get_runtime().block_on(async move { relay.lock().await.aclose().await });
        });
    }

    #[cfg_attr(test, mutants::skip)]
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let relay = Arc::clone(&self.relay);
        future_into_py(py, async move {
            relay.lock().await.aclose().await;
            Ok::<(), PyErr>(())
        })
    }
}
