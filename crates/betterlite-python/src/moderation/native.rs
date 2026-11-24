use super::convert::{ConvertError, ModerationCallWire, TextCallWire};
use super::error::{CallFailure, SessionError, class_error};
use super::moderate::moderate;
use super::session::{Entry, TextSession};
use super::text::call as text_call;
use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

type Blobs<'py> = Bound<'py, PyList>;
type Pending = PyResult<Py<PyAny>>;

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

    // the synchronous text completion, this thread waiting on the runtime with the GIL released
    fn text_completion(&self, py: Python<'_>, text: &str, _blobs: &Blobs<'_>) -> PyResult<String> {
        let call: TextCallWire = wire(text)?;
        let outcome = py.detach(|| get_runtime().block_on(text_call(&self.session, call)));
        encoded(outcome)
    }

    fn atext_completion(&self, py: Python<'_>, text: &str, _blobs: &Blobs<'_>) -> Pending {
        let call: TextCallWire = wire(text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move { encoded(text_call(&session, call).await) })
            .map(Bound::unbind)
    }

    fn moderation(&self, py: Python<'_>, text: &str, _blobs: &Blobs<'_>) -> PyResult<String> {
        let call: ModerationCallWire = wire(text)?;
        let outcome =
            py.detach(|| get_runtime().block_on(moderate(&self.session, call, Entry::Sync)));
        encoded(outcome)
    }

    fn amoderation(&self, py: Python<'_>, text: &str, _blobs: &Blobs<'_>) -> Pending {
        let call: ModerationCallWire = wire(text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            encoded(moderate(&session, call, Entry::Async).await)
        })
        .map(Bound::unbind)
    }
}

fn wire<T>(text: &str) -> PyResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(text)
        .map_err(|source| PyErr::from(SessionError::from(ConvertError::Json { source })))
}

fn encoded<T>(outcome: Result<T, CallFailure>) -> PyResult<String>
where
    T: Serialize,
{
    match outcome {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(class_error(&failure.0)),
    }
}
