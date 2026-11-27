use std::sync::Arc;

use betterlite_models::decisions::Json;
use pyo3::PyErr;
use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert::{ConvertError, parse};
use super::error::raise;
use super::session::{Pending, PendingOperation, SessionCore};

mod tests;

// the attached-file list every call takes; a vector store file call only reads whether it is empty
type Blobs<'py> = Bound<'py, PyList>;

// main.py's module-level entry points: one session owning the configs and the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeVectorFileSession {
    core: Arc<SessionCore>,
}

#[pymethods]
impl NativeVectorFileSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    // the synchronous create: the file joined to the store, blocking this thread with the GIL released
    fn create(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        settled(
            py,
            &self.core,
            PendingOperation::Create(parse(text, blobs.len())?),
            false,
        )
    }

    // the synchronous list: the store's files, read with the five query fields the handler takes
    fn list(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        settled(
            py,
            &self.core,
            PendingOperation::List(parse(text, blobs.len())?),
            false,
        )
    }

    // the synchronous retrieve: the file object, with no body or query of its own
    fn retrieve(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        settled(
            py,
            &self.core,
            PendingOperation::Retrieve(parse(text, blobs.len())?),
            false,
        )
    }

    // the synchronous content retrieve: the parsed text of the file, never its object
    fn retrieve_content(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        settled(
            py,
            &self.core,
            PendingOperation::RetrieveContent(parse(text, blobs.len())?),
            false,
        )
    }

    // the synchronous update: the attributes replace the file's own, and an empty dict is sent
    fn update(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        settled(
            py,
            &self.core,
            PendingOperation::Update(parse(text, blobs.len())?),
            false,
        )
    }

    // the synchronous delete: the store drops the file and answers with the deletion object
    fn delete(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<String> {
        settled(
            py,
            &self.core,
            PendingOperation::Delete(parse(text, blobs.len())?),
            false,
        )
    }

    // the awaitable create: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn acreate(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        awaitable(
            py,
            Arc::clone(&self.core),
            PendingOperation::Create(parse(text, blobs.len())?),
        )
    }

    // the awaitable list: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn alist(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        awaitable(
            py,
            Arc::clone(&self.core),
            PendingOperation::List(parse(text, blobs.len())?),
        )
    }

    // the awaitable retrieve: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn aretrieve(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        awaitable(
            py,
            Arc::clone(&self.core),
            PendingOperation::Retrieve(parse(text, blobs.len())?),
        )
    }

    // the awaitable content retrieve: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn aretrieve_content(
        &self,
        py: Python<'_>,
        text: &str,
        blobs: &Blobs<'_>,
    ) -> PyResult<Py<PyAny>> {
        awaitable(
            py,
            Arc::clone(&self.core),
            PendingOperation::RetrieveContent(parse(text, blobs.len())?),
        )
    }

    // the awaitable update: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn aupdate(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        awaitable(
            py,
            Arc::clone(&self.core),
            PendingOperation::Update(parse(text, blobs.len())?),
        )
    }

    // the awaitable delete: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn adelete(&self, py: Python<'_>, text: &str, blobs: &Blobs<'_>) -> PyResult<Py<PyAny>> {
        awaitable(
            py,
            Arc::clone(&self.core),
            PendingOperation::Delete(parse(text, blobs.len())?),
        )
    }
}

// The synchronous path: one attempt on the runtime, with the GIL released for as long as the call runs
fn settled(
    py: Python<'_>,
    core: &SessionCore,
    operation: PendingOperation,
    asynchronous: bool,
) -> PyResult<String> {
    let pending = Pending {
        operation,
        asynchronous,
    };
    let outcome = py.detach(|| get_runtime().block_on(core.run(pending)));
    let body = outcome.map_err(|error| raise(py, &error, &core.redactor))?;
    to_text(&body)
}

// The awaitable path: the same call driven by the asyncio loop's runtime and handed back as a coroutine
fn awaitable(
    py: Python<'_>,
    core: Arc<SessionCore>,
    operation: PendingOperation,
) -> PyResult<Py<PyAny>> {
    let pending = Pending {
        operation,
        asynchronous: true,
    };
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
