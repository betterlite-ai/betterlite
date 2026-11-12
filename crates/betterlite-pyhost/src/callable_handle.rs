use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyTuple};
use pyo3::{Bound, Py, PyAny, PyResult, Python};

use crate::blocking_work::WorkTracker;
use crate::call_error::{CallError, PythonError};
use crate::host::PythonHost;
use crate::json_values::{JsonObject, JsonValue, from_object, object_type_name, to_object};

const WAIT_GRACE: Duration = Duration::from_secs(1);

type PythonArguments = (Vec<Py<PyAny>>, BTreeMap<String, Py<PyAny>>);

#[derive(Debug)]
pub struct CallableHandle {
    pub object: Py<PyAny>,
    pub is_async: bool,
}

#[expect(
    clippy::too_many_arguments,
    reason = "a Python call carries its arguments and keywords beside the host, tracker and deadline"
)]
impl CallableHandle {
    pub fn new(host: &PythonHost, object: Py<PyAny>) -> Result<Self, CallError> {
        let is_async = host
            .attach(|py| {
                detects_async(py, object.bind(py))
                    .map_err(|error| PythonError::from_pyerr(py, &error))
            })
            .map_err(CallError::Python)?;
        Ok(Self { object, is_async })
    }

    #[must_use]
    pub fn name(&self, host: &PythonHost) -> String {
        host.attach(|py| {
            let object = self.object.bind(py);
            object
                .getattr_opt("__qualname__")
                .ok()
                .flatten()
                .map_or_else(
                    || object_type_name(object),
                    |qualified| {
                        qualified
                            .extract::<String>()
                            .unwrap_or_else(|_error| object_type_name(object))
                    },
                )
        })
    }

    pub fn attr(&self, host: &PythonHost, name: &str) -> Result<Self, CallError> {
        let object = host
            .attach(|py| {
                self.object
                    .bind(py)
                    .getattr(name)
                    .map(Bound::unbind)
                    .map_err(|error| PythonError::from_pyerr(py, &error))
            })
            .map_err(CallError::Python)?;
        Self::new(host, object)
    }

    pub async fn call(
        &self,
        host: &PythonHost,
        tracker: &WorkTracker,
        deadline: Duration,
        args: Vec<JsonValue>,
        kwargs: JsonObject,
    ) -> Result<JsonValue, CallError> {
        let (positional, keywords) = json_arguments(host, &args, &kwargs)?;
        let result = self
            .call_raw(host, tracker, deadline, positional, keywords)
            .await?;
        host.attach(|py| from_object(result.bind(py)))
            .map_err(CallError::Value)
    }

    pub async fn call_raw(
        &self,
        host: &PythonHost,
        tracker: &WorkTracker,
        deadline: Duration,
        args: Vec<Py<PyAny>>,
        kwargs: BTreeMap<String, Py<PyAny>>,
    ) -> Result<Py<PyAny>, CallError> {
        let started = Instant::now();
        let function = host.attach(|py| self.object.clone_ref(py));
        let outcome = tracker
            .run(host, deadline, move |py| {
                call_once(py, &function, args, kwargs)
            })
            .await
            .map_err(CallError::Work)??;
        match outcome {
            CallOutcome::Ready(value) => Ok(value),
            CallOutcome::Awaitable(awaitable) => {
                let remaining = deadline.saturating_sub(started.elapsed());
                tracker
                    .run(host, remaining.saturating_add(WAIT_GRACE), move |py| {
                        event_loop::wait_on_loop(py, &awaitable, remaining, deadline)
                    })
                    .await
                    .map_err(CallError::Work)?
            }
        }
    }
}

enum CallOutcome {
    Ready(Py<PyAny>),
    Awaitable(Py<PyAny>),
}

fn call_once(
    py: Python<'_>,
    function: &Py<PyAny>,
    args: Vec<Py<PyAny>>,
    kwargs: BTreeMap<String, Py<PyAny>>,
) -> Result<CallOutcome, CallError> {
    let positional = PyTuple::new(py, args).map_err(|error| PythonError::from_pyerr(py, &error))?;
    let keywords = PyDict::new(py);
    for (name, value) in kwargs {
        keywords
            .set_item(name, value)
            .map_err(|error| PythonError::from_pyerr(py, &error))?;
    }
    let result = function
        .bind(py)
        .call(&positional, Some(&keywords))
        .map_err(|error| PythonError::from_pyerr(py, &error))?;
    if is_awaitable(py, &result)? {
        return Ok(CallOutcome::Awaitable(result.unbind()));
    }
    Ok(CallOutcome::Ready(result.unbind()))
}

fn is_awaitable(py: Python<'_>, value: &Bound<'_, PyAny>) -> Result<bool, CallError> {
    py.import("inspect")
        .and_then(|inspect| inspect.call_method1("isawaitable", (value,)))
        .and_then(|flag| flag.is_truthy())
        .map_err(|error| CallError::Python(PythonError::from_pyerr(py, &error)))
}

fn detects_async(py: Python<'_>, object: &Bound<'_, PyAny>) -> PyResult<bool> {
    let inspect = py.import("inspect")?;
    if inspect
        .call_method1("iscoroutinefunction", (object,))?
        .is_truthy()?
    {
        return Ok(true);
    }
    if inspect.call_method1("isfunction", (object,))?.is_truthy()? {
        return Ok(false);
    }
    match object.getattr_opt("__call__")? {
        Some(call) => inspect
            .call_method1("iscoroutinefunction", (&call,))?
            .is_truthy(),
        None => Ok(false),
    }
}

fn json_arguments(
    host: &PythonHost,
    args: &[JsonValue],
    kwargs: &JsonObject,
) -> Result<PythonArguments, CallError> {
    host.attach(|py| {
        call_arguments(py, args, kwargs).map_err(|error| PythonError::from_pyerr(py, &error))
    })
    .map_err(CallError::Python)
}

fn call_arguments(
    py: Python<'_>,
    args: &[JsonValue],
    kwargs: &JsonObject,
) -> PyResult<PythonArguments> {
    let positional = args
        .iter()
        .map(|value| to_object(py, value).map(Bound::unbind))
        .collect::<PyResult<Vec<_>>>()?;
    let keywords = kwargs
        .iter()
        .map(|(name, value)| Ok((name.clone(), to_object(py, value)?.unbind())))
        .collect::<PyResult<BTreeMap<_, _>>>()?;
    Ok((positional, keywords))
}

mod event_loop;
mod tests;
