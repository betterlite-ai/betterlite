use super::convert::{GenaiCall, parse, reject_files};
use super::session::{Entry, GenaiCore, Outcome};
use super::stream::NativeGenaiStream;
use pyo3::prelude::*;
use pyo3::types::PyString;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::sync::Arc;
// The session litellm.google_genai's four entry points call, owning the clients every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeGenaiSession {
    core: Arc<GenaiCore>,
}
#[pymethods]
impl NativeGenaiSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            core: Arc::new(GenaiCore::build(settings)?),
        })
    }
    // generate_content: one attempt on the runtime, blocking this thread with the GIL released
    fn generate_content(
        &self,
        py: Python<'_>,
        payload: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Py<PyAny>> {
        let call = wire(payload, blobs)?;
        let core = Arc::clone(&self.core);
        let outcome = py.detach(|| get_runtime().block_on(core.call(call, Entry::Blocking)));
        answer(py, outcome?)
    }
    // agenerate_content: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn agenerate_content<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = wire(payload, blobs)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = Box::pin(core.call(call, Entry::Await)).await?;
            Python::attach(|attached| answer(attached, outcome))
        })
    }
    // generate_content_stream: the blocking iterator LiteLLM's indexable stream object exposes
    fn generate_content_stream(
        &self,
        py: Python<'_>,
        payload: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Py<PyAny>> {
        let call = wire(payload, blobs)?;
        let core = Arc::clone(&self.core);
        let outcome = py.detach(|| get_runtime().block_on(core.call(call, Entry::BlockingStream)));
        answer(py, outcome?)
    }
    // agenerate_content_stream: the awaitable that answers the async iterator after one round trip
    fn agenerate_content_stream<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = wire(payload, blobs)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = Box::pin(core.call(call, Entry::AwaitStream)).await?;
            Python::attach(|attached| answer(attached, outcome))
        })
    }
}
// the payload text and its file bytes as one call; a $file marker has no place in a generate content body
fn wire(payload: &str, mut blobs: Vec<Vec<u8>>) -> Result<GenaiCall, PyErr> {
    reject_files(payload, &mut blobs)?;
    Ok(parse(payload)?)
}
// the answer Python reads: the response text it loads into a dict, or the stream object it iterates
fn answer(py: Python<'_>, outcome: Outcome) -> PyResult<Py<PyAny>> {
    match outcome {
        Outcome::Text(text) => Ok(PyString::new(py, &text).into_any().unbind()),
        Outcome::Stream(kind) => Ok(Py::new(py, NativeGenaiStream::new(kind))?.into_any()),
    }
}
