use super::answer::{awaiting, outcome_object};
use super::convert::{GenaiCall, parse, reject_files};
use super::session::{Entry, GenaiCore};
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::get_runtime;
use std::sync::Arc;
mod tests;
type Blobs = Vec<Vec<u8>>;
type Awaiting<'py> = PyResult<Bound<'py, PyAny>>;
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
    #[pyo3(name = "generate_content")]
    fn content(&self, py: Python<'_>, payload: &str, blobs: Blobs) -> PyResult<Py<PyAny>> {
        let call = wire(payload, blobs)?;
        let core = Arc::clone(&self.core);
        let outcome = py.detach(|| get_runtime().block_on(core.call(call, Entry::Blocking)))?;
        outcome_object(py, outcome)
    }
    // generate_content_stream: the same attempt, answering the blocking iterator
    #[pyo3(name = "generate_content_stream")]
    fn content_stream(&self, py: Python<'_>, payload: &str, blobs: Blobs) -> PyResult<Py<PyAny>> {
        let call = wire(payload, blobs)?;
        let core = Arc::clone(&self.core);
        let outcome =
            py.detach(|| get_runtime().block_on(core.call(call, Entry::BlockingStream)))?;
        outcome_object(py, outcome)
    }
    // agenerate_content: the same call on the asyncio loop's runtime, handed back as a coroutine
    #[pyo3(name = "agenerate_content")]
    fn acontent<'py>(&self, py: Python<'py>, payload: &str, blobs: Blobs) -> Awaiting<'py> {
        let call = wire(payload, blobs)?;
        let core = Arc::clone(&self.core);
        awaiting(py, core, Entry::Await, call)
    }
    // agenerate_content_stream: the awaitable that answers the async iterator after one round trip
    #[pyo3(name = "agenerate_content_stream")]
    fn acontent_stream<'py>(&self, py: Python<'py>, payload: &str, blobs: Blobs) -> Awaiting<'py> {
        let call = wire(payload, blobs)?;
        let core = Arc::clone(&self.core);
        awaiting(py, core, Entry::AwaitStream, call)
    }
}
// the payload text and its file bytes as one call; a $file marker has no place in a generate content body
fn wire(payload: &str, mut blobs: Vec<Vec<u8>>) -> Result<GenaiCall, PyErr> {
    reject_files(payload, &mut blobs)?;
    Ok(parse(payload)?)
}
