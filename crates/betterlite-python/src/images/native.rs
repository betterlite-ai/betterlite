use super::convert::{ConvertError, ImageAnswer, ImageCallWire, parse};
use super::edit::edit;
use super::error::{CallFailure, NativeImageError, SessionError, class_error};
use super::generate::generate;
use super::session::{Entry, ImageSession};
use super::variation::variation;
use betterlite_sdk::call_files::CallFile;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::sync::Arc;

type Blobs = Vec<Vec<u8>>;
type Pending = PyResult<Py<PyAny>>;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeImageSession {
    session: Arc<ImageSession>,
}

#[pymethods]
impl NativeImageSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            session: Arc::new(ImageSession::build(settings)?),
        })
    }

    // image_generation: one attempt on the runtime, blocking this thread with the GIL released
    fn generate(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let (call, _files) = wire(payload, &mut blobs)?;
        encoded(py.detach(|| get_runtime().block_on(generate(&self.session, call))))
    }

    fn agenerate(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let (call, _files) = wire(payload, &mut blobs)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move { encoded(generate(&session, call).await) })
            .map(Bound::unbind)
    }

    fn variation(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let (call, files) = wire(payload, &mut blobs)?;
        let outcome = py
            .detach(|| get_runtime().block_on(variation(&self.session, call, files, Entry::Sync)));
        encoded(outcome)
    }

    fn avariation(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let (call, files) = wire(payload, &mut blobs)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            encoded(variation(&session, call, files, Entry::Async).await)
        })
        .map(Bound::unbind)
    }

    fn edit(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let (call, files) = wire(payload, &mut blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(edit(&self.session, call, files)));
        encoded(outcome)
    }

    fn aedit(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let (call, files) = wire(payload, &mut blobs)?;
        let session = Arc::clone(&self.session);
        future_into_py(
            py,
            async move { encoded(edit(&session, call, files).await) },
        )
        .map(Bound::unbind)
    }
}

fn wire(
    payload: &str,
    blobs: &mut [Vec<u8>],
) -> PyResult<(ImageCallWire, Vec<(String, CallFile)>)> {
    parse(payload, blobs).map_err(|source| PyErr::from(SessionError::from(source)))
}

fn encoded(outcome: Result<ImageAnswer, CallFailure>) -> PyResult<String> {
    match outcome {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(class_error(&failure)),
    }
}
