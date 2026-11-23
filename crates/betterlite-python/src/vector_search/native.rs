use std::sync::Arc;

use betterlite_models::decisions::Json;
use pyo3::PyErr;
use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert::{ConvertError, CreateWire, SearchWire, parse};
use super::error::raise;
use super::session::{Pending, SessionCore};

mod tests;

// the attached-file list every call takes; a vector store call only reads whether it is empty
type Blobs<'py> = Bound<'py, PyList>;

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
    fn create(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        let wire: CreateWire = parse(text, blobs.len())?;
        let pending = Pending::Create(wire);
        settled(py, &self.core, pending)
    }

    // the synchronous search, with the query sent as the text or the list the caller gave
    fn search(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        let wire: SearchWire = parse(text, blobs.len())?;
        let pending = Pending::Search(wire);
        settled(py, &self.core, pending)
    }

    // the awaitable create: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn acreate(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        let wire: CreateWire = parse(text, blobs.len())?;
        let pending = Pending::Create(wire);
        awaitable(py, Arc::clone(&self.core), pending)
    }

    // the awaitable search: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn asearch(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        let wire: SearchWire = parse(text, blobs.len())?;
        let pending = Pending::Search(wire);
        awaitable(py, Arc::clone(&self.core), pending)
    }
}

// The synchronous path: one attempt on the runtime, with the GIL released for as long as the call runs
fn settled(py: Python<'_>, core: &SessionCore, pending: Pending) -> PyResult<String> {
    let outcome = py.detach(|| get_runtime().block_on(core.run(pending)));
    let body = outcome.map_err(|error| raise(py, &error, &core.redactor))?;
    to_text(&body)
}

// The awaitable path: the same call driven by the asyncio loop's runtime and handed back as a coroutine
fn awaitable(py: Python<'_>, core: Arc<SessionCore>, pending: Pending) -> PyResult<Py<PyAny>> {
    let coroutine = future_into_py(py, async move {
        let body = Box::pin(core.run(pending))
            .await
            .map_err(|error| Python::attach(|attached| raise(attached, &error, &core.redactor)))?;
        to_text(&body)
    })?;
    Ok(coroutine.unbind())
}

// The answer is JSON text, the one shape every native family hands back to Python
fn to_text(body: &Json) -> PyResult<String> {
    match serde_json::to_string(body) {
        Ok(text) => Ok(text),
        Err(source) => Err(PyErr::from(ConvertError::Encode { source })),
    }
}
