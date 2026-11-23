use super::call;
use super::convert::{EmbeddingCallWire, parse};
use super::error::{SessionError, call_failure};
use super::response::serialize_answer;
use super::session::EmbeddingSession;
use pyo3::prelude::*;
use pyo3::types::PyList;
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
    fn embedding(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyList>,
    ) -> PyResult<String> {
        let wire: EmbeddingCallWire = parse(request).map_err(SessionError::from)?;
        py.detach(|| get_runtime().block_on(answer_text(&self.session, wire)))
    }
    // the awaitable embedding: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn aembedding<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyList>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: EmbeddingCallWire = parse(request).map_err(SessionError::from)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move { answer_text(&session, wire).await })
    }
}
// both entry points answer the same JSON text or raise the same NativeEmbeddingError
async fn answer_text(session: &EmbeddingSession, wire: EmbeddingCallWire) -> PyResult<String> {
    let answer = Box::pin(call::call(session, wire))
        .await
        .map_err(|failure| call_failure(&failure))?;
    serialize_answer(&answer).map_err(|error| PyErr::from(SessionError::from(error)))
}
