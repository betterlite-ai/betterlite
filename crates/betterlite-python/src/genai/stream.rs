use super::convert::NativeErrorPayload;
use super::error::payload_error;
use betterlite_core::google_genai::streaming::{
    AsyncGoogleGenAiStream, GenAiStreamError, GoogleGenAiStream,
};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::sync::Arc;
use tokio::sync::Mutex;
mod tests;
// The two iterator classes LiteLLM answers a stream with: the sync entry point's and the async one's.
pub(crate) enum StreamKind {
    Blocking(Box<GoogleGenAiStream>),
    Asynchronous(Box<AsyncGoogleGenAiStream>),
}
// One stream object Python iterates with for or async for; dropping the last chunk closes the connection.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeGenaiStream {
    kind: Arc<Mutex<Option<StreamKind>>>,
}
impl From<StreamKind> for NativeGenaiStream {
    fn from(kind: StreamKind) -> Self {
        Self {
            kind: Arc::new(Mutex::new(Some(kind))),
        }
    }
}
#[pymethods]
impl NativeGenaiStream {
    // the blocking iterator the synchronous entry point returns, reading one event per call
    fn next_chunk(&self, py: Python<'_>) -> PyResult<Option<Py<PyBytes>>> {
        let kind = Arc::clone(&self.kind);
        let next = py.detach(move || get_runtime().block_on(pull(kind)));
        let event =
            next.map_err(|error| payload_error(&NativeErrorPayload::from_stream(&error)))?;
        Ok(event.map(|data| PyBytes::new(py, &data).unbind()))
    }
    fn anext_chunk<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let kind = Arc::clone(&self.kind);
        future_into_py(py, async move {
            let event = pull(kind)
                .await
                .map_err(|error| payload_error(&NativeErrorPayload::from_stream(&error)))?;
            Ok(Python::attach(|attached| {
                event.map(|data| PyBytes::new(attached, &data).unbind())
            }))
        })
    }
    fn close(&self, py: Python<'_>) {
        py.detach(|| {
            get_runtime().block_on(async {
                drop(Arc::clone(&self.kind).lock().await.take());
            });
        });
    }
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.close(py);
        future_into_py(py, async { Ok::<(), PyErr>(()) })
    }
}
// one event of the stream, None once it is spent, and the failure that ended it
async fn pull(kind: Arc<Mutex<Option<StreamKind>>>) -> Result<Option<Vec<u8>>, GenAiStreamError> {
    let mut guard = kind.lock().await;
    let Some(stream) = guard.as_mut() else {
        return Ok(None);
    };
    let pulled = match stream {
        StreamKind::Blocking(blocking) => blocking.next(),
        StreamKind::Asynchronous(asynchronous) => match asynchronous.next_event().await {
            Ok(event) => event.map(Ok),
            Err(error) => Some(Err(error)),
        },
    };
    if !matches!(pulled, Some(Ok(_))) {
        guard.take();
    }
    drop(guard);
    match pulled {
        Some(Ok(event)) => Ok(Some(event)),
        Some(Err(error)) => Err(error),
        None => Ok(None),
    }
}
