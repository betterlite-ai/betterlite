use super::call::{Entry, call};
use super::convert::{ConvertError, RerankCallWire};
use super::error::{SessionError, call_failure};
use super::session::RerankSession;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::sync::Arc;

mod tests;

// main.py's rerank entry points, one session owning the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeRerankSession {
    session: Arc<RerankSession>,
}

#[cfg_attr(test, mutants::skip)]
#[pymethods]
impl NativeRerankSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let session = RerankSession::build(settings)?;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    // the synchronous rerank: one call on the runtime, this thread waiting with the GIL released
    fn rerank(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyList>,
    ) -> PyResult<String> {
        py.detach(|| {
            let wire = bounded_wire(request)?;
            get_runtime().block_on(answer_text(&self.session, wire, Entry::Sync))
        })
    }

    // the awaitable rerank: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn arerank<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'py, PyList>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire = bounded_wire(request)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            answer_text(&session, wire, Entry::Async).await
        })
    }
}

// both entry points answer the same JSON text or raise the same NativeRerankError
#[cfg_attr(test, mutants::skip)]
async fn answer_text(
    session: &RerankSession,
    wire: RerankCallWire,
    entry: Entry,
) -> PyResult<String> {
    match Box::pin(call(session, wire, entry)).await {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(call_failure(failure)),
    }
}

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 100_000;
const MAX_HEADERS: usize = 256;

fn bounded_wire(request: &str) -> Result<RerankCallWire, SessionError> {
    if request.len() > MAX_REQUEST_BYTES {
        return Err(too_large("bytes", request.len(), MAX_REQUEST_BYTES));
    }
    let wire: RerankCallWire = serde_json::from_str(request)
        .map_err(|source| SessionError::from(ConvertError::Json { source }))?;
    let documents = wire.arguments.documents.len();
    if documents > MAX_DOCUMENTS {
        return Err(too_large("documents", documents, MAX_DOCUMENTS));
    }
    let mut headers = wire.settings.headers.as_ref().map_or(0, Headers::len);
    if let Some(Json::Object(named)) = wire.kwargs.get("headers") {
        headers = headers.saturating_add(named.len());
    }
    if headers > MAX_HEADERS {
        return Err(too_large("headers", headers, MAX_HEADERS));
    }
    Ok(wire)
}

fn too_large(field: &str, found: usize, limit: usize) -> SessionError {
    SessionError::Limit(format!(
        "the rerank request has {found} {field} (limit {limit})"
    ))
}
