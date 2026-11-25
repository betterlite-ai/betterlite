use std::sync::Arc;

use betterlite_core::api::passthrough_call::{PassthroughBody, PassthroughReply};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert::call_args;
use super::error::native_error;
use super::session::SessionCore;
use super::stream::NativePassthroughStream;

mod tests;

// One Python passthrough session: the client every call of this instance goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativePassthroughSession {
    core: Arc<SessionCore>,
}

#[pymethods]
impl NativePassthroughSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    // the synchronous route: one call on the runtime, blocking this thread with the GIL released
    fn call(&self, py: Python<'_>, request: &str, mut blobs: Vec<Vec<u8>>) -> PyResult<Py<PyAny>> {
        let args = call_args(request, &mut blobs).map_err(PyErr::from)?;
        let reply = py
            .detach(|| get_runtime().block_on(self.core.call(args)))
            .map_err(|error| native_error(*error))?;
        answer(py, reply)
    }

    // the awaitable route, handed back as a coroutine the caller awaits
    fn acall<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        mut blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let args = call_args(request, &mut blobs).map_err(PyErr::from)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let reply = core
                .call(args)
                .await
                .map_err(|error| native_error(*error))?;
            Python::attach(|attached| answer(attached, reply))
        })
    }
}

// A finished call is the whole body, or the stream of chunks, with the upstream status and headers.
fn answer(py: Python<'_>, reply: PassthroughReply) -> PyResult<Py<PyAny>> {
    let body = match reply.body {
        PassthroughBody::Full(bytes) => PyBytes::new(py, bytes.as_ref()).into_any().unbind(),
        PassthroughBody::Stream(stream) => {
            Py::new(py, NativePassthroughStream::from(stream))?.into_any()
        }
    };
    Ok((reply.status, reply.headers, body)
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
