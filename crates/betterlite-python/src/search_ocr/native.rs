use std::future::Future;
use std::sync::Arc;

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert::{ConvertError, CostCall, OcrCall, SearchCall};
use super::error::{NativeFailure, SessionError};
use super::session::SessionCore;

// The native coroutine an awaitable entry hands to asyncio.
type Coroutine<'py> = PyResult<Bound<'py, PyAny>>;

// litellm.search, its cost function and litellm.ocr: one session owning the environment every call reads.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeSearchSession {
    core: Arc<SessionCore>,
}

#[pymethods]
impl NativeSearchSession {
    #[new]
    fn new(settings: &str) -> Result<Self, SessionError> {
        Ok(Self {
            core: Arc::new(SessionCore::build(settings)?),
        })
    }

    // the synchronous search: one attempt on the runtime, blocking this thread with the GIL released
    fn search(&self, py: Python<'_>, request: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let call: SearchCall =
            serde_json::from_str(request).map_err(|source| ConvertError::Json { source })?;
        py.detach(|| get_runtime().block_on(self.core.search(call)))
            .map_err(NativeFailure::into_pyerr)
    }

    // the awaitable search: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn asearch<'py>(&self, py: Python<'py>, request: &str, _blobs: Vec<Vec<u8>>) -> Coroutine<'py> {
        let call: SearchCall =
            serde_json::from_str(request).map_err(|source| ConvertError::Json { source })?;
        let core = Arc::clone(&self.core);
        awaited(py, move || async move { core.search(call).await })
    }

    // search_provider_cost_per_query: a number pair, or the model-not-mapped outcome Python raises
    fn cost(&self, py: Python<'_>, request: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let call: CostCall =
            serde_json::from_str(request).map_err(|source| ConvertError::Json { source })?;
        py.detach(|| get_runtime().block_on(self.core.cost(call)))
            .map_err(NativeFailure::into_pyerr)
    }

    // the synchronous OCR call, with the caller's file bytes beside the request text
    fn ocr(&self, py: Python<'_>, request: &str, blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let call: OcrCall =
            serde_json::from_str(request).map_err(|source| ConvertError::Json { source })?;
        py.detach(|| get_runtime().block_on(self.core.ocr(call, blobs)))
            .map_err(NativeFailure::into_pyerr)
    }

    // the awaitable OCR call: the same binding on the asyncio loop's runtime, with its file bytes
    fn aocr<'py>(&self, py: Python<'py>, request: &str, blobs: Vec<Vec<u8>>) -> Coroutine<'py> {
        let call: OcrCall =
            serde_json::from_str(request).map_err(|source| ConvertError::Json { source })?;
        let core = Arc::clone(&self.core);
        awaited(py, move || async move { core.ocr(call, &blobs).await })
    }
}

// An awaitable entry hands the loop the coroutine its closure builds and raises a failed call in it.
fn awaited<'py, F, Fut>(py: Python<'py>, answer: F) -> Coroutine<'py>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<String, NativeFailure>> + Send + 'static,
{
    future_into_py(py, async move {
        answer().await.map_err(NativeFailure::into_pyerr)
    })
}
