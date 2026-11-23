use super::call;
use super::convert::{EmbeddingCallWire, parse};
use super::error::{NativeEmbeddingError, call_failure};
use super::response::serialize_answer;
use super::session::EmbeddingSession;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::sync::Arc;
// main.py's embedding entry points, one session owning the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeEmbeddingSession {
    session: Arc<EmbeddingSession>,
}
#[pymethods]
impl NativeEmbeddingSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let session = EmbeddingSession::build(settings)?;
        Ok(Self {
            session: Arc::new(session),
        })
    }
    // the synchronous embedding: one call on the runtime, this thread waiting with the GIL released
    fn embedding(&self, py: Python<'_>, request: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let wire: EmbeddingCallWire = parse(request)?;
        let answer = py.detach(|| get_runtime().block_on(call::call(&self.session, wire)));
        let answer = answer.map_err(|failure| call_failure(&failure))?;
        serialize_answer(&answer).map_err(PyErr::from)
    }
    // the awaitable embedding: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn aembedding<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: EmbeddingCallWire = parse(request)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let answer = Box::pin(call::call(&session, wire))
                .await
                .map_err(|failure| call_failure(&failure))?;
            serialize_answer(&answer).map_err(PyErr::from)
        })
    }
}
