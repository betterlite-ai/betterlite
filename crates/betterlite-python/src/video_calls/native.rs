use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::Serialize;
use std::sync::Arc;

use super::convert::{ConvertError, Operation, parse, request};
use super::error::RunError;
use super::operations::remix_list;
use super::operations::{self, Input, Mode, Parsed};
use super::session::VideoSession;

type Blobs = Vec<Vec<u8>>;
type Pending = PyResult<Py<PyAny>>;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeVideoSession {
    session: Arc<VideoSession>,
}

#[pymethods]
impl NativeVideoSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            session: Arc::new(VideoSession::build(settings)?),
        })
    }

    // video_generation: the handler answers asynchronously, so the runtime waits for it here with the GIL released
    fn generation(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let input = prepared(&self.session, Operation::Generate, payload, &mut blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(operations::generate(input, Mode::Sync)));
        encoded(outcome)
    }

    fn ageneration(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, Operation::Generate, payload, &mut blobs)?;
        let future = async move { encoded(operations::generate(input, Mode::Async).await) };
        future_into_py(py, future).map(Bound::unbind)
    }

    // video_content: the downloaded bytes themselves, never re-encoded
    fn content(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<Vec<u8>> {
        let input = prepared(&self.session, Operation::Content, payload, &mut blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(operations::content(input)));
        outcome.map_err(PyErr::from)
    }

    fn acontent(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, Operation::Content, payload, &mut blobs)?;
        let future = async move { operations::content(input).await.map_err(PyErr::from) };
        future_into_py(py, future).map(Bound::unbind)
    }

    // video_status: whatever the caller named is the provider, and the answer is the refreshed object
    fn status(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let input = prepared(&self.session, Operation::Status, payload, &mut blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(operations::status(input)));
        encoded(outcome)
    }

    fn astatus(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, Operation::Status, payload, &mut blobs)?;
        let future = async move { encoded(operations::status(input).await) };
        future_into_py(py, future).map(Bound::unbind)
    }

    // video_remix: its transport blocks the thread itself, so no runtime may be entered around it
    fn remix(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let input = prepared(&self.session, Operation::Remix, payload, &mut blobs)?;
        let outcome = py.detach(|| remix_list::remix(input));
        encoded(outcome)
    }

    fn aremix(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, Operation::Remix, payload, &mut blobs)?;
        let future = async move { encoded(remix_list::aremix(input).await) };
        future_into_py(py, future).map(Bound::unbind)
    }

    // video_list: the page as the provider sent it, or the objects a mock_response named
    fn list(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let input = prepared(&self.session, Operation::List, payload, &mut blobs)?;
        let outcome = py.detach(|| remix_list::list(input));
        encoded(outcome)
    }

    fn alist(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, Operation::List, payload, &mut blobs)?;
        let future = async move { encoded(remix_list::alist(input).await) };
        future_into_py(py, future).map(Bound::unbind)
    }
}

// the payload's wire, the one request its operation asks for, and the input_reference file it carried
fn parsed(operation: Operation, payload: &str, blobs: &mut Blobs) -> Result<Parsed, RunError> {
    let (wire, reference) = parse(payload, blobs)?;
    let requests = request(operation, &wire, reference.clone())?;
    Ok((wire, requests, reference))
}

// one parsed payload bound to a share of the session, so Python's own future can own it
fn prepared(
    session: &Arc<VideoSession>,
    operation: Operation,
    payload: &str,
    blobs: &mut Blobs,
) -> Result<Input, RunError> {
    let shared = Arc::clone(session);
    let pieces = parsed(operation, payload, blobs)?;
    Ok(Input::new(shared, pieces))
}

// the object, the page or the mock's objects as the JSON text Python's TypeAdapter reads back
fn encoded<T>(outcome: Result<T, RunError>) -> PyResult<String>
where
    T: Serialize,
{
    match outcome {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(RunError::from(ConvertError::Encode { source }))),
        Err(error) => Err(PyErr::from(error)),
    }
}

mod tests;
