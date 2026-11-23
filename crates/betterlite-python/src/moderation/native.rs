use super::convert::{ConvertError, ModerationCallWire, TextCallWire};
use super::error::{SessionError, class_error};
use super::moderate::moderate;
use super::session::{Entry, TextSession};
use super::text::call as text_call;
use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::sync::Arc;

// main.py's four entry points, one session owning the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeTextSession {
    session: Arc<TextSession>,
}

#[pymethods]
impl NativeTextSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let session = TextSession::build(settings)?;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    // the synchronous text completion: one call on the runtime, this thread waiting with the GIL released
    fn text_completion(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyList>,
    ) -> PyResult<String> {
        py.detach(|| {
            let wire: TextCallWire = serde_json::from_str(request)
                .map_err(|source| SessionError::from(ConvertError::Json { source }))?;
            get_runtime().block_on(text_text(&self.session, wire))
        })
    }

    // the awaitable text completion: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn atext_completion<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyList>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: TextCallWire = serde_json::from_str(request)
            .map_err(|source| SessionError::from(ConvertError::Json { source }))?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move { text_text(&session, wire).await })
    }

    // the synchronous moderation: one call on the runtime, this thread waiting with the GIL released
    fn moderation(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyList>,
    ) -> PyResult<String> {
        py.detach(|| {
            let wire: ModerationCallWire = serde_json::from_str(request)
                .map_err(|source| SessionError::from(ConvertError::Json { source }))?;
            get_runtime().block_on(moderate_text(&self.session, wire, Entry::Sync))
        })
    }

    // the awaitable moderation: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn amoderation<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyList>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: ModerationCallWire = serde_json::from_str(request)
            .map_err(|source| SessionError::from(ConvertError::Json { source }))?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            moderate_text(&session, wire, Entry::Async).await
        })
    }
}

// both text entry points answer the same JSON text or raise the same NativeTextError
async fn text_text(session: &TextSession, wire: TextCallWire) -> PyResult<String> {
    match Box::pin(text_call(session, wire)).await {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(class_error(&failure.0)),
    }
}

async fn moderate_text(
    session: &TextSession,
    wire: ModerationCallWire,
    entry: Entry,
) -> PyResult<String> {
    match Box::pin(moderate(session, wire, entry)).await {
        Ok(response) => serde_json::to_string(&response)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(class_error(&failure.0)),
    }
}
