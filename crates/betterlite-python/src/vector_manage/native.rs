use std::sync::Arc;

use betterlite_models::decisions::Json;
use pyo3::PyErr;
use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert::{ConvertError, DeleteWire, ListWire, RetrieveWire, UpdateWire, parse};
use super::error::raise;
use super::session::{Pending, SessionCore, SystemClock, SystemIds};

// the attached-file list every call takes; a vector store call only reads whether it is empty
type Blobs<'py> = Bound<'py, PyList>;

// main.py's module-level entry points: one session owning the configs and the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeVectorManageSession {
    core: Arc<SessionCore>,
}

// Each sync method runs one call to completion; each `a` twin hands the same call back as a coroutine.
#[pymethods]
impl NativeVectorManageSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings, Arc::new(SystemClock::new()), Arc::new(SystemIds))?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    fn retrieve(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        let wire: RetrieveWire = parse(text, blobs.len())?;
        settled(py, &self.core, Pending::Retrieve(wire))
    }

    fn list(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        let wire: ListWire = parse(text, blobs.len())?;
        settled(py, &self.core, Pending::List(wire))
    }

    fn update(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        let wire: UpdateWire = parse(text, blobs.len())?;
        settled(py, &self.core, Pending::Update(wire))
    }

    fn delete(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        let wire: DeleteWire = parse(text, blobs.len())?;
        settled(py, &self.core, Pending::Delete(wire))
    }

    fn aretrieve(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        let wire: RetrieveWire = parse(text, blobs.len())?;
        awaitable(py, Arc::clone(&self.core), Pending::Aretrieve(wire))
    }

    fn alist(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        let wire: ListWire = parse(text, blobs.len())?;
        awaitable(py, Arc::clone(&self.core), Pending::Alist(wire))
    }

    fn aupdate(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        let wire: UpdateWire = parse(text, blobs.len())?;
        awaitable(py, Arc::clone(&self.core), Pending::Aupdate(wire))
    }

    fn adelete(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        let wire: DeleteWire = parse(text, blobs.len())?;
        awaitable(py, Arc::clone(&self.core), Pending::Adelete(wire))
    }
}

// The synchronous path: one attempt on the runtime, with the GIL released for as long as the call runs
fn settled(py: Python<'_>, core: &SessionCore, pending: Pending) -> PyResult<String> {
    let outcome = py.detach(|| get_runtime().block_on(core.run(pending)));
    let body = outcome.map_err(|error| raise(py, &error))?;
    to_text(&body)
}

// The awaitable path: the same call driven by the asyncio loop's runtime and handed back as a coroutine
fn awaitable(py: Python<'_>, core: Arc<SessionCore>, pending: Pending) -> PyResult<Py<PyAny>> {
    let coroutine = future_into_py(py, async move {
        let body = Box::pin(core.run(pending))
            .await
            .map_err(|error| Python::attach(|attached| raise(attached, &error)))?;
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
