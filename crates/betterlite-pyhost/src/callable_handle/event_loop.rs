use std::ffi::CStr;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::Builder;
use std::time::Duration;

use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::types::{
    PyAnyMethods as _, PyDict, PyDictMethods as _, PyModule, PyTuple, PyTupleMethods as _,
};
use pyo3::{Bound, Py, PyAny, PyErr, PyResult, Python};

use crate::call_error::{CallError, PythonError};

const LOOP_SLOT: &str = "_betterlite_pyhost_loop";
const LOOP_THREAD_NAME: &str = "betterlite-py-loop";
// asyncio stops its loop when a task raises SystemExit or KeyboardInterrupt, so the guard hands those back as values
const GUARD_CODE: &CStr = c"async def guard(awaitable):\n    try:\n        return (await awaitable, None)\n    except (SystemExit, KeyboardInterrupt) as error:\n        return (None, error)\n";

pub(super) fn wait_on_loop(
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
        Err(_wait_error) => settled_outcome(py, &future, after),
    }
}

// Python 3.11+ makes the builtin TimeoutError both the class a hook raises and the one result() raises when the wait runs out, so the future's state tells a hook failure from the deadline.
fn settled_outcome(
    py: Python<'_>,
    future: &Bound<'_, PyAny>,
    after: Duration,
) -> Result<Py<PyAny>, CallError> {
    let timed_out = !future
        .call_method0("done")
        .and_then(|done| done.is_truthy())
        .map_err(|source| CallError::Python(PythonError::from_pyerr(py, &source)))?;
    if timed_out {
        drop(
            future
                .call_method0("cancel")
                .map_err(|source| CallError::Python(PythonError::from_pyerr(py, &source)))?,
        );
        return Err(CallError::TimedOut { after });
    }
    let outcome = future
        .call_method0("result")
        .map_err(|raised| CallError::Python(PythonError::from_pyerr(py, &raised)))?;
    guarded_outcome(py, &outcome)
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
