use super::call::{Entry, call};
use super::convert::{ConvertError, EmbeddingCallWire};
use super::error::{SessionError, call_failure};
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
        py.detach(|| {
            let wire: EmbeddingCallWire = serde_json::from_str(request)
                .map_err(|source| SessionError::from(ConvertError::Json { source }))?;
            get_runtime().block_on(answer_text(&self.session, wire, Entry::Sync))
        })
    }
    // the awaitable embedding: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn aembedding<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyList>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: EmbeddingCallWire = serde_json::from_str(request)
            .map_err(|source| SessionError::from(ConvertError::Json { source }))?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            answer_text(&session, wire, Entry::Async).await
        })
    }
}
// both entry points answer the same JSON text or raise the same NativeEmbeddingError
async fn answer_text(
    session: &EmbeddingSession,
    wire: EmbeddingCallWire,
    entry: Entry,
) -> PyResult<String> {
    match Box::pin(call(session, wire, entry)).await {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(call_failure(&failure)),
    }
}
