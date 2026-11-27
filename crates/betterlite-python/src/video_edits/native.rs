use std::sync::Arc;

use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg as owned_kwarg;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::Serialize;

use super::convert::requests::Build;
use super::convert::{ConvertError, parse};
use super::error::RunError;
use super::operations::{self, Input};
use super::session::EditSession;

type Blobs = Vec<Vec<u8>>;
type Pending = PyResult<Py<PyAny>>;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeVideoEditSession {
    session: Arc<EditSession>,
}

#[pymethods]
impl NativeVideoEditSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            session: Arc::new(EditSession::build(settings)?),
        })
    }

    // video_create_character: the mock, the refused provider, then the multipart upload
    fn create_character(
        &self,
        py: Python<'_>,
        payload: &str,
        mut blobs: Blobs,
    ) -> PyResult<String> {
        let input = prepared(&self.session, payload, &mut blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(operations::create_character(input)));
        encoded(outcome)
    }

    fn acreate_character(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, payload, &mut blobs)?;
        let future = async move { encoded(operations::create_character(input).await) };
        future_into_py(py, future).map(Bound::unbind)
    }

    // video_get_character: the same flow for the one lookup
    fn get_character(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let input = prepared(&self.session, payload, &mut blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(operations::get_character(input)));
        encoded(outcome)
    }

    fn aget_character(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, payload, &mut blobs)?;
        let future = async move { encoded(operations::get_character(input).await) };
        future_into_py(py, future).map(Bound::unbind)
    }

    // video_edit: a file sends multipart, and a video id sends JSON
    fn edit(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let input = prepared(&self.session, payload, &mut blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(operations::edit(input)));
        encoded(outcome)
    }

    fn aedit(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, payload, &mut blobs)?;
        let future = async move { encoded(operations::edit(input).await) };
        future_into_py(py, future).map(Bound::unbind)
    }

    // video_extension: always the JSON form, with seconds passed on as the string it was
    fn extension(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let input = prepared(&self.session, payload, &mut blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(operations::extension(input)));
        encoded(outcome)
    }

    fn aextension(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let input = prepared(&self.session, payload, &mut blobs)?;
        let future = async move { encoded(operations::extension(input).await) };
        future_into_py(py, future).map(Bound::unbind)
    }
}

// the wire, the one request this operation asks for, and the video file it carried
fn prepared<Request>(
    session: &Arc<EditSession>,
    payload: &str,
    blobs: &mut Blobs,
) -> Result<Input<Request>, RunError>
where
    Request: Build,
{
    let (wire, video) = parse(payload, blobs)?;
    let request = Request::build(&wire.arguments, video.as_ref())?;
    Ok(Input::new(Arc::clone(session), (wire, request, video)))
}

// the object as the JSON text Python's TypeAdapter reads back
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

// _utils.py asks the same question of a keyword name that LiteLLM's is_litellm_owned_kwarg answers
#[pyfunction]
// ast-grep-ignore: no-one-line-functions - the core predicate is exposed to the Python package under its own name
pub(crate) fn is_litellm_owned_kwarg(name: &str) -> bool {
    owned_kwarg(name)
}

mod tests;
