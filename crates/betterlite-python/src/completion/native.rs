use super::convert::{CallWire, OutcomeParts, ProbeCall, TokenCall, parse};
use super::error::{call_error, validation_error};
use super::session::{ProbeAnswer, ProbeCheck, SessionCore};
use super::stream::NativeChatStream;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyString;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::sync::Arc;
mod tests;
// main.py's module-level entry points, one session owning the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeSession {
    core: Arc<SessionCore>,
}
#[pymethods]
impl NativeSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }
    // the synchronous completion: one attempt on the runtime, blocking this thread with the GIL released
    fn completion(&self, py: Python<'_>, request: &str) -> PyResult<Py<PyAny>> {
        let wire: CallWire = parse(request)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.call(wire, false)));
        let parts = outcome.map_err(|error| call_error(&error))?;
        answer(py, parts)
    }
    // the awaitable completion: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn acompletion<'py>(&self, py: Python<'py>, request: &str) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let parts = core
                .call(wire, true)
                .await
                .map_err(|error| call_error(&error))?;
            Python::attach(|attached| answer(attached, parts))
        })
    }
    fn count_tokens(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let call: TokenCall = parse(request)?;
        let counted = py.detach(|| get_runtime().block_on(self.core.count(call)));
        let response = counted.map_err(|error| call_error(&error))?;
        serde_json::to_string(&response).map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }
    // utils.check_valid_key returns False for every failure, an authentication one included
    fn check_valid_key(&self, py: Python<'_>, request: &str) -> PyResult<bool> {
        let call: ProbeCall = parse(request)?;
        let run = py.detach(|| get_runtime().block_on(self.core.probe(&call, ProbeCheck::Key)));
        let proved = run.map_err(|error| call_error(&error))?;
        Ok(proved == ProbeAnswer::Valid)
    }
    // utils.valid_model raises BadRequestError(message="", model=model, llm_provider="") on any failure
    fn valid_model(&self, py: Python<'_>, request: &str) -> PyResult<()> {
        let call: ProbeCall = parse(request)?;
        let run = py.detach(|| get_runtime().block_on(self.core.probe(&call, ProbeCheck::Model)));
        let proved = run.map_err(|error| call_error(&error))?;
        if proved == ProbeAnswer::Invalid {
            return Err(validation_error(&call.model));
        }
        Ok(())
    }
}
// A finished call is either the response text or the stream of chunk texts Python iterates.
fn answer(py: Python<'_>, parts: OutcomeParts) -> PyResult<Py<PyAny>> {
    match parts {
        OutcomeParts::Response(text) => Ok(PyString::new(py, &text).into_any().unbind()),
        OutcomeParts::Stream(chunks) => Ok(Py::new(py, NativeChatStream::new(chunks))?.into_any()),
    }
}
