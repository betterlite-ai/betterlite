use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::Builder;
use std::time::{Duration, Instant};

use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::types::{
    PyAnyMethods as _, PyDict, PyDictMethods as _, PyModule, PyTuple, PyTupleMethods as _,
};
use pyo3::{Bound, Py, PyAny, PyErr, PyResult, Python};

use crate::blocking_work::WorkTracker;
use crate::call_error::{CallError, PythonError};
use crate::host::PythonHost;
use crate::json_values::{JsonObject, JsonValue, from_object, object_type_name, to_object};

const LOOP_SLOT: &str = "_betterlite_pyhost_loop";
const LOOP_THREAD_NAME: &str = "betterlite-py-loop";
const WAIT_GRACE: Duration = Duration::from_secs(1);
// asyncio stops its loop when a task raises SystemExit or KeyboardInterrupt, so the guard hands those back as values
const GUARD_CODE: &CStr = c"async def guard(awaitable):\n    try:\n        return (await awaitable, None)\n    except (SystemExit, KeyboardInterrupt) as error:\n        return (None, error)\n";

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
        let (positional, keywords) = host
            .attach(|py| {
                call_arguments(py, &args, &kwargs)
                    .map_err(|error| PythonError::from_pyerr(py, &error))
            })
            .map_err(CallError::Python)?;
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
                        wait_on_loop(py, &awaitable, remaining, deadline)
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

fn wait_on_loop(
    py: Python<'_>,
    awaitable: &Py<PyAny>,
    remaining: Duration,
    after: Duration,
) -> Result<Py<PyAny>, CallError> {
    let runtime =
        shared_loop(py).map_err(|error| CallError::Python(PythonError::from_pyerr(py, &error)))?;
    let future = py
        .import("asyncio")
        .and_then(|asyncio| {
            let guarded = runtime.guard.bind(py).call1((awaitable.bind(py),))?;
            asyncio.call_method1(
                "run_coroutine_threadsafe",
                (guarded, runtime.loop_object.bind(py)),
            )
        })
        .map_err(|error| CallError::Python(PythonError::from_pyerr(py, &error)))?;
    // Future.result releases the GIL while it waits, so the loop thread runs the coroutine.
    match future.call_method1("result", (remaining.as_secs_f64(),)) {
        Ok(outcome) => guarded_outcome(py, &outcome),
        Err(error) => {
            if !is_timeout_error(py, &error) {
                return Err(CallError::Python(PythonError::from_pyerr(py, &error)));
            }
            drop(future.call_method0("cancel").map_err(|cancel_error| {
                CallError::Python(PythonError::from_pyerr(py, &cancel_error))
            })?);
            Err(CallError::TimedOut { after })
        }
    }
}

fn guarded_outcome(py: Python<'_>, outcome: &Bound<'_, PyAny>) -> Result<Py<PyAny>, CallError> {
    let pair = outcome.cast::<PyTuple>().map_err(|_error| {
        CallError::Python(PythonError::from_pyerr(
            py,
            &PyTypeError::new_err("the event loop guard returned something other than a pair"),
        ))
    })?;
    let error = pair
        .get_item(1)
        .map_err(|source| CallError::Python(PythonError::from_pyerr(py, &source)))?;
    if error.is_none() {
        let value = pair
            .get_item(0)
            .map_err(|source| CallError::Python(PythonError::from_pyerr(py, &source)))?;
        return Ok(value.unbind());
    }
    let raised = PyErr::from_value(error);
    Err(CallError::Python(PythonError::from_pyerr(py, &raised)))
}

fn is_timeout_error(py: Python<'_>, error: &PyErr) -> bool {
    py.import("concurrent.futures")
        .and_then(|futures| futures.getattr("TimeoutError"))
        .is_ok_and(|timeout| error.is_instance(py, &timeout))
}

struct AsyncLoop {
    loop_object: Py<PyAny>,
    guard: Py<PyAny>,
}

fn shared_loop(py: Python<'_>) -> PyResult<AsyncLoop> {
    let sys = py.import("sys")?;
    let slot = if let Some(slot) = sys.getattr_opt(LOOP_SLOT)? {
        slot.cast_into::<PyDict>().map_err(PyErr::from)?
    } else {
        let created = PyDict::new(py);
        sys.setattr(LOOP_SLOT, &created)?;
        created
    };
    if let Some(existing) = slot.get_item("loop")? {
        return runtime_from_slot(&slot, existing.unbind());
    }
    let lock = if let Some(lock) = slot.get_item("start_lock")? {
        lock
    } else {
        let created = py.import("threading")?.call_method0("Lock")?;
        slot.set_item("start_lock", &created)?;
        created
    };
    lock.call_method0("acquire")?;
    let outcome = start_loop(py, &slot);
    drop(lock.call_method0("release")?);
    runtime_from_slot(&slot, outcome?)
}

fn runtime_from_slot(slot: &Bound<'_, PyDict>, loop_object: Py<PyAny>) -> PyResult<AsyncLoop> {
    let Some(guard) = slot.get_item("guard")? else {
        return Err(PyRuntimeError::new_err(
            "the shared event loop has no guard function",
        ));
    };
    Ok(AsyncLoop {
        loop_object,
        guard: guard.unbind(),
    })
}

fn start_loop(py: Python<'_>, slot: &Bound<'_, PyDict>) -> PyResult<Py<PyAny>> {
    if let Some(existing) = slot.get_item("loop")? {
        return Ok(existing.unbind());
    }
    let guard = PyModule::from_code(
        py,
        GUARD_CODE,
        c"<betterlite-pyhost>",
        c"_betterlite_pyhost_guard",
    )?
    .getattr("guard")?
    .unbind();
    slot.set_item("guard", &guard)?;
    let (sender, receiver) = sync_channel(1);
    Builder::new()
        .name(LOOP_THREAD_NAME.to_owned())
        .spawn(move || run_loop_thread(sender))
        .map_err(|source| {
            PyRuntimeError::new_err(format!(
                "could not start the {LOOP_THREAD_NAME} thread: {source}"
            ))
        })?;
    let answer = py.detach(move || receiver.recv());
    let loop_object = match answer {
        Ok(Ok(loop_object)) => loop_object,
        Ok(Err(error)) => return Err(error),
        Err(_) => {
            return Err(PyRuntimeError::new_err(format!(
                "the {LOOP_THREAD_NAME} thread ended before its loop started"
            )));
        }
    };
    slot.set_item("loop", &loop_object)?;
    Ok(loop_object)
}

fn run_loop_thread(sender: SyncSender<PyResult<Py<PyAny>>>) {
    Python::attach(move |py| {
        let loop_object = match create_loop(py) {
            Ok(loop_object) => loop_object,
            Err(error) => {
                drop(sender.send(Err(error)));
                return;
            }
        };
        if sender.send(Ok(loop_object.clone_ref(py))).is_err() {
            return;
        }
        drop(loop_object.bind(py).call_method0("run_forever"));
    });
}

fn create_loop(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let asyncio = py.import("asyncio")?;
    let loop_object = asyncio.call_method0("new_event_loop")?;
    asyncio.call_method1("set_event_loop", (&loop_object,))?;
    py.import("threading")?
        .call_method0("current_thread")?
        .setattr("name", LOOP_THREAD_NAME)?;
    Ok(loop_object.unbind())
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

mod tests;
