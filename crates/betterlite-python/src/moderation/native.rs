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

enum Call {
    Text(TextCallWire),
    Moderation(ModerationCallWire, Entry),
}

impl NativeTextSession {
    // the synchronous answer on the runtime, this thread waiting with the GIL released
    fn run(&self, py: Python<'_>, call: Call) -> PyResult<String> {
        py.detach(|| get_runtime().block_on(answer(&self.session, call)))
    }

    fn spawn(&self, py: Python<'_>, call: Call) -> Pending {
        let session = Arc::clone(&self.session);
        future_into_py(py, async move { answer(&session, call).await }).map(Bound::unbind)
    }
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

    fn text_completion(&self, py: Python<'_>, text: &str, _blobs: &Blobs<'_>) -> PyResult<String> {
        self.run(py, Call::Text(wire(text)?))
    }

    fn atext_completion(&self, py: Python<'_>, text: &str, _blobs: &Blobs<'_>) -> Pending {
        self.spawn(py, Call::Text(wire(text)?))
    }

    fn moderation(&self, py: Python<'_>, text: &str, _blobs: &Blobs<'_>) -> PyResult<String> {
        self.run(py, Call::Moderation(wire(text)?, Entry::Sync))
    }

    fn amoderation(&self, py: Python<'_>, text: &str, _blobs: &Blobs<'_>) -> Pending {
        self.spawn(py, Call::Moderation(wire(text)?, Entry::Async))
    }
}

fn wire<T: DeserializeOwned>(text: &str) -> PyResult<T> {
    serde_json::from_str(text)
        .map_err(|source| PyErr::from(SessionError::from(ConvertError::Json { source })))
}

async fn answer(session: &TextSession, call: Call) -> PyResult<String> {
    match call {
        Call::Text(wire) => encoded(text_call(session, wire).await),
        Call::Moderation(wire, entry) => encoded(moderate(session, wire, entry).await),
    }
}

fn encoded<T: Serialize>(outcome: Result<T, CallFailure>) -> PyResult<String> {
    match outcome {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(class_error(&failure.0)),
    }
}
