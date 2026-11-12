#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::time::{Duration, Instant};

use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyModule};
use pyo3::{Py, PyAny};
use tokio::time::sleep;

use super::CallableHandle;
use crate::blocking_work::WorkTracker;
use crate::call_error::CallError;
use crate::host::{HostConfig, PythonHost};
use crate::json_values::{JsonObject, JsonValue};

const WAIT_LIMIT: Duration = Duration::from_secs(10);

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn code_module(host: &PythonHost, name: &CStr, code: &CStr) -> Py<PyAny> {
    host.attach(|py| {
        PyModule::from_code(py, code, c"hook_test.py", name)
            .unwrap()
            .into_any()
            .unbind()
    })
}

fn attribute(host: &PythonHost, module: &Py<PyAny>, name: &str) -> Py<PyAny> {
    host.attach(|py| module.bind(py).getattr(name).unwrap().unbind())
}

fn handle_of(host: &PythonHost, object: Py<PyAny>) -> CallableHandle {
    CallableHandle::new(host, object).unwrap()
}

fn tracker() -> WorkTracker {
    WorkTracker::new(2, Duration::from_secs(30))
}

fn field<'value>(value: &'value JsonValue, name: &str) -> &'value JsonValue {
    match value {
        JsonValue::Object(entries) => entries.get(name).unwrap(),
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)) => panic!("expected an object, got {other:?}"),
    }
}

const ADD_CODE: &CStr = c"def add(a, b=0):\n    return a + b\n";

#[tokio::test]
async fn a_sync_function_returns_its_result_through_call() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", ADD_CODE);
    let hook = handle_of(&host, attribute(&host, &module, "add"));
    let kwargs = JsonObject::from_iter([("b".to_owned(), JsonValue::from(2_i64))]);
    let result = hook
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![JsonValue::from(1_i64)],
            kwargs,
        )
        .await
        .unwrap();
    assert_eq!(result, JsonValue::from(3_i64));
}

const DOUBLE_CODE: &CStr =
    c"import asyncio\n\nasync def double(value):\n    await asyncio.sleep(0)\n    return value * 2\n";

#[tokio::test]
async fn an_async_function_returns_its_result_through_call() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", DOUBLE_CODE);
    let hook = handle_of(&host, attribute(&host, &module, "double"));
    assert!(hook.is_async);
    let result = hook
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![JsonValue::from(21_i64)],
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(result, JsonValue::from(42_i64));
}

const COLLECT_CODE: &CStr =
    c"async def collect(*args, **kwargs):\n    return {'args': list(args), 'kwargs': kwargs}\n";

#[tokio::test]
async fn positional_and_keyword_arguments_reach_the_callable() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", COLLECT_CODE);
    let hook = handle_of(&host, attribute(&host, &module, "collect"));
    let kwargs = JsonObject::from_iter([("c".to_owned(), JsonValue::from(3_i64))]);
    let result = hook
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![JsonValue::from(1_i64), JsonValue::from("two")],
            kwargs,
        )
        .await
        .unwrap();
    let expected = JsonObject::from_iter([
        (
            "args".to_owned(),
            JsonValue::Array(vec![JsonValue::from(1_i64), JsonValue::from("two")]),
        ),
        (
            "kwargs".to_owned(),
            JsonValue::Object(JsonObject::from_iter([(
                "c".to_owned(),
                JsonValue::from(3_i64),
            )])),
        ),
    ]);
    assert_eq!(result, JsonValue::Object(expected));
}

const LATER_CODE: &CStr =
    c"async def _later(value):\n    return value + 1\n\ndef later(value):\n    return _later(value)\n";

#[tokio::test]
async fn a_plain_function_returning_a_coroutine_object_is_awaited() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", LATER_CODE);
    let hook = handle_of(&host, attribute(&host, &module, "later"));
    assert!(!hook.is_async);
    let result = hook
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![JsonValue::from(5_i64)],
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(result, JsonValue::from(6_i64));
}

const ASYNC_CALL_CODE: &CStr =
    c"class Handler:\n    async def __call__(self, value):\n        return value + 10\n";

#[tokio::test]
async fn a_callable_instance_with_an_async_call_is_async() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", ASYNC_CALL_CODE);
    let instance = host.attach(|py| {
        module
            .bind(py)
            .getattr("Handler")
            .unwrap()
            .call0()
            .unwrap()
            .unbind()
    });
    let hook = handle_of(&host, instance);
    assert!(hook.is_async);
    let result = hook
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![JsonValue::from(1_i64)],
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(result, JsonValue::from(11_i64));
}

const SYNC_CALL_CODE: &CStr =
    c"class SyncHandler:\n    def __call__(self, value):\n        return value - 10\n";

#[tokio::test]
async fn a_callable_instance_with_a_sync_call_is_sync() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", SYNC_CALL_CODE);
    let instance = host.attach(|py| {
        module
            .bind(py)
            .getattr("SyncHandler")
            .unwrap()
            .call0()
            .unwrap()
            .unbind()
    });
    let hook = handle_of(&host, instance);
    assert!(!hook.is_async);
    let result = hook
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![JsonValue::from(1_i64)],
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(result, JsonValue::from(-9_i64));
}

const COUNTER_CODE: &CStr = c"class Counter:\n    def __init__(self, start):\n        self.value = start\n\n    def next(self):\n        self.value += 1\n        return self.value\n";

#[tokio::test]
async fn attr_returns_a_handle_to_an_attribute() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", COUNTER_CODE);
    let instance = host.attach(|py| {
        module
            .bind(py)
            .getattr("Counter")
            .unwrap()
            .call1((10_i64,))
            .unwrap()
            .unbind()
    });
    let counter = handle_of(&host, instance);
    let next = counter.attr(&host, "next").unwrap();
    let first = next
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap();
    let second = next
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(first, JsonValue::from(11_i64));
    assert_eq!(second, JsonValue::from(12_i64));
}

#[tokio::test]
async fn name_returns_the_qualified_name() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", ADD_CODE);
    let hook = handle_of(&host, attribute(&host, &module, "add"));
    assert_eq!(hook.name(&host), "add");
}

const PARTIAL_CODE: &CStr = c"import functools\n\nhandler = functools.partial(len)\n";

#[tokio::test]
async fn name_falls_back_to_the_type_name_when_there_is_no_qualified_name() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", PARTIAL_CODE);
    let hook = handle_of(&host, attribute(&host, &module, "handler"));
    assert_eq!(hook.name(&host), "partial");
}

const VECTOR_CODE: &CStr = c"class Vector:\n    def __init__(self, x, y):\n        self.x = x\n        self.y = y\n\n    def total(self):\n        return self.x + self.y\n\ndef describe(vector):\n    return vector.total()\n";

#[tokio::test]
async fn call_raw_passes_python_objects_and_returns_one() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", VECTOR_CODE);
    let instance = host.attach(|py| {
        module
            .bind(py)
            .getattr("Vector")
            .unwrap()
            .call1((3_i64, 4_i64))
            .unwrap()
            .unbind()
    });
    let hook = handle_of(&host, attribute(&host, &module, "describe"));
    let result = hook
        .call_raw(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![instance],
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let total = host.attach(|py| result.bind(py).extract::<i64>().unwrap());
    assert_eq!(total, 7);
}

const SLOW_CODE: &CStr = c"import asyncio\n\nstate = {'cancelled': False}\n\nasync def slow():\n    try:\n        await asyncio.sleep(30)\n    except asyncio.CancelledError:\n        state['cancelled'] = True\n        raise\n    return 'late'\n\nasync def quick():\n    return 'quick'\n";

async fn wait_for_cancellation(host: &PythonHost, module: &Py<PyAny>) {
    let clock = Instant::now();
    while clock.elapsed() < WAIT_LIMIT {
        let cancelled = host.attach(|py| {
            let state = module.bind(py).getattr("state").unwrap();
            let entries = state.cast_into::<PyDict>().unwrap();
            let cancelled = entries.get_item("cancelled").unwrap().unwrap();
            cancelled.extract::<bool>().unwrap()
        });
        if cancelled {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("the coroutine was not cancelled");
}

#[tokio::test]
async fn an_async_call_that_passes_the_deadline_is_cancelled_and_the_loop_serves_the_next_call() {
    let host = start_host();
    let module = code_module(&host, c"hook_test", SLOW_CODE);
    let slow = handle_of(&host, attribute(&host, &module, "slow"));
    let clock = Instant::now();
    let outcome = slow
        .call(
            &host,
            &tracker(),
            Duration::from_millis(100),
            Vec::new(),
            JsonObject::new(),
        )
        .await;
    assert!(
        clock.elapsed() < WAIT_LIMIT,
        "the call waited past its deadline"
    );
    match outcome {
        Err(CallError::TimedOut { after }) => assert_eq!(after, Duration::from_millis(100)),
        other => panic!("expected TimedOut, got {other:?}"),
    }
    wait_for_cancellation(&host, &module).await;
    let quick = handle_of(&host, attribute(&host, &module, "quick"));
    let result = quick
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(result, JsonValue::from("quick"));
}

const WHERE_CODE: &CStr = c"import threading\n\nasync def where():\n    return {'ident': threading.get_ident(), 'name': threading.current_thread().name}\n";

#[tokio::test]
async fn every_handle_awaits_on_the_one_named_loop_thread() {
    let host = start_host();
    let first_module = code_module(&host, c"hook_test", WHERE_CODE);
    let second_module = code_module(&host, c"hook_test", WHERE_CODE);
    let first = handle_of(&host, attribute(&host, &first_module, "where"));
    let second = handle_of(&host, attribute(&host, &second_module, "where"));
    let first_where = first
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap();
    let second_where = second
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        field(&first_where, "name"),
        &JsonValue::from("betterlite-py-loop")
    );
    assert_eq!(field(&first_where, "ident"), field(&second_where, "ident"));
}
