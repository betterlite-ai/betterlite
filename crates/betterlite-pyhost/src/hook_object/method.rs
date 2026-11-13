use std::time::Duration;

use pyo3::types::PyAnyMethods;
use pyo3::{Bound, Py, PyAny};

use crate::blocking_work::WorkTracker;
use crate::callable_handle::CallableHandle;
use crate::host::PythonHost;

use super::{HookError, RawMethodCall};

pub(crate) fn callable_attribute(object: &Bound<'_, PyAny>, name: &str) -> Option<Py<PyAny>> {
    object
        .getattr_opt(name)
        .ok()
        .flatten()
        .filter(PyAnyMethods::is_callable)
        .map(Bound::unbind)
}

pub(crate) async fn invoke_raw(
    handle: CallableHandle,
    host: &PythonHost,
    tracker: &WorkTracker,
    deadline: Duration,
    call: RawMethodCall<'_>,
) -> Result<Py<PyAny>, HookError> {
    handle
        .call_raw(host, tracker, deadline, call.args, call.kwargs)
        .await
        .map_err(HookError::Call)
}

mod tests;
