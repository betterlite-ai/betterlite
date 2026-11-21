use super::convert::{CallWire, OutcomeParts, ProbeCall, TokenCall, parse};
use super::error::{call_error, request_error, session_error, validation_error};
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
        let core = SessionCore::build(settings).map_err(session_error)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }
    // the synchronous completion: one attempt on the runtime, blocking this thread with the GIL released
    fn completion(&self, py: Python<'_>, request: &str) -> PyResult<Py<PyAny>> {
        let wire: CallWire = parse(request).map_err(request_error)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.call(wire, false)));
        match outcome {
            Ok(parts) => answer(py, parts),
            Err(error) => Err(call_error(&error)),
        }
    }
    // the awaitable completion: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn acompletion<'py>(&self, py: Python<'py>, request: &str) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallWire = parse(request).map_err(request_error)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, run_async(core, wire))
    }
    fn count_tokens(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let call: TokenCall = parse(request).map_err(request_error)?;
        let counted = py.detach(|| get_runtime().block_on(self.core.count(call)));
        let response = counted.map_err(|error| call_error(&error))?;
        serde_json::to_string(&response).map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }
    // utils.check_valid_key returns False for every failure, an authentication one included
    fn check_valid_key(&self, py: Python<'_>, request: &str) -> PyResult<bool> {
        let call: ProbeCall = parse(request).map_err(request_error)?;
        let proved = run_probe(&self.core, py, &call, ProbeCheck::Key)?;
        Ok(matches!(proved, ProbeAnswer::Valid))
    }
    // utils.valid_model raises BadRequestError(message="", model=model, llm_provider="") on any failure
    fn valid_model(&self, py: Python<'_>, request: &str) -> PyResult<()> {
        let call: ProbeCall = parse(request).map_err(request_error)?;
        let proved = run_probe(&self.core, py, &call, ProbeCheck::Model)?;
        if let ProbeAnswer::Invalid = proved {
            return Err(validation_error(&call.model));
        }
        Ok(())
    }
}
// the awaitable form of one call on the runtime, so the event loop keeps running while it waits
async fn run_async(core: Arc<SessionCore>, wire: CallWire) -> PyResult<Py<PyAny>> {
    let parts = core
        .call(wire, true)
        .await
        .map_err(|error| call_error(&error))?;
    Python::attach(|py| answer(py, parts))
}
// one probe run on the runtime, with the GIL released
fn run_probe(
    core: &SessionCore,
    py: Python<'_>,
    call: &ProbeCall,
    check: ProbeCheck,
) -> PyResult<ProbeAnswer> {
    let proved = py.detach(|| get_runtime().block_on(core.probe(call, check)));
    proved.map_err(|error| call_error(&error))
}
// A finished call is either the response text or the stream of chunk texts Python iterates.
fn answer(py: Python<'_>, parts: OutcomeParts) -> PyResult<Py<PyAny>> {
    match parts {
        OutcomeParts::Response(text) => Ok(PyString::new(py, &text).into_any().unbind()),
        OutcomeParts::Stream(chunks) => Ok(Py::new(py, NativeChatStream::new(chunks))?.into_any()),
    }
}
