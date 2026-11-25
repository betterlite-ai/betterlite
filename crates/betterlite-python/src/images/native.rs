use super::convert::{ConvertError, ImageAnswer, parse};
use super::edit::edit;
use super::error::{CallFailure, SessionError, class_error};
use super::generate::generate;
use super::session::ImageSession;
use super::variation::variation;
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
        let (call, _files) =
            parse(payload, &mut blobs).map_err(|source| PyErr::from(SessionError::from(source)))?;
        encoded(py.detach(|| get_runtime().block_on(generate(&self.session, call))))
    }

    fn agenerate(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let (call, _files) =
            parse(payload, &mut blobs).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move { encoded(generate(&session, call).await) })
            .map(Bound::unbind)
    }

    fn variation(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let (call, files) =
            parse(payload, &mut blobs).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let outcome =
            py.detach(|| get_runtime().block_on(variation(&self.session, call, files, false)));
        encoded(outcome)
    }

    fn avariation(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let (call, files) =
            parse(payload, &mut blobs).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let session = Arc::clone(&self.session);
        let future = async move { encoded(variation(&session, call, files, true).await) };
        future_into_py(py, future).map(Bound::unbind)
    }

    fn edit(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let (call, files) =
            parse(payload, &mut blobs).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let outcome = py.detach(|| get_runtime().block_on(edit(&self.session, call, files)));
        encoded(outcome)
    }

    fn aedit(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let (call, files) =
            parse(payload, &mut blobs).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let session = Arc::clone(&self.session);
        let future = async move { encoded(edit(&session, call, files).await) };
        future_into_py(py, future).map(Bound::unbind)
    }
}

fn encoded(outcome: Result<ImageAnswer, CallFailure>) -> PyResult<String> {
    match outcome {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(class_error(&failure)),
    }
}

mod tests;
