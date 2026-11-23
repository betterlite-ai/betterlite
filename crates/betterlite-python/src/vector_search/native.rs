use std::sync::Arc;

use betterlite_models::decisions::Json;
use pyo3::PyErr;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert::{ConvertError, CreateWire, SearchWire, parse};
use super::error::raise;
use super::session::SessionCore;

mod tests;

// main.py's module-level entry points: one session owning the configs and the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeVectorSearchSession {
    core: Arc<SessionCore>,
}

#[pymethods]
impl NativeVectorSearchSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    // the synchronous create: one attempt on the runtime, blocking this thread with the GIL released
    fn create(&self, py: Python<'_>, request: &str, blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let wire: CreateWire = parse(request, &blobs)?;
        let answer = py.detach(|| get_runtime().block_on(self.core.create(&wire)));
        let body = answer.map_err(|error| raise(py, &error, &self.core.redactor))?;
        to_text(&body)
    }

    // the synchronous search, with the query sent as the text or the list the caller gave
    fn search(&self, py: Python<'_>, request: &str, blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let wire: SearchWire = parse(request, &blobs)?;
        let answer = py.detach(|| get_runtime().block_on(self.core.search(&wire)));
        let body = answer.map_err(|error| raise(py, &error, &self.core.redactor))?;
        to_text(&body)
    }

    // the awaitable create: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn acreate<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: CreateWire = parse(request, &blobs)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let body = Box::pin(core.create(&wire))
                .await
                .map_err(|error| Python::attach(|py| raise(py, &error, &core.redactor)))?;
            to_text(&body)
        })
    }

    // the awaitable search: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn asearch<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: SearchWire = parse(request, &blobs)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let body = Box::pin(core.search(&wire))
                .await
                .map_err(|error| Python::attach(|py| raise(py, &error, &core.redactor)))?;
            to_text(&body)
        })
    }
}

// The answer is JSON text, the one shape every native family hands back to Python
fn to_text(body: &Json) -> PyResult<String> {
    match serde_json::to_string(body) {
        Ok(text) => Ok(text),
        Err(source) => Err(PyErr::from(ConvertError::Encode { source })),
    }
}
