#![cfg(test)]

use std::ffi::CStr;
use std::time::Duration;

use pyo3::types::{PyAnyMethods as _, PyModule};

use crate::blocking_work::WorkTracker;
use crate::hook_object::{HookError, HookObject, MethodCall};
use crate::host::{HostConfig, PythonHost};
use crate::instance_loader::LoadedObject;
use crate::json_values::{JsonObject, JsonValue};

const HANDLER_CODE: &CStr = c"state = {'calls': 0}\n\nclass Handler:\n    def __init__(self, base):\n        self.base = base\n\n    def add(self, value):\n        return self.base + value\n\n    async def add_async(self, value):\n        return self.base + value\n\n    def touch(self):\n        state['calls'] += 1\n        return state['calls']\n\n    label = 'plain'\n\ndef recorded():\n    return state['calls']\n";

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn tracker() -> WorkTracker {
    WorkTracker::new(2, Duration::from_secs(30))
}

fn handler_fixture(host: &PythonHost) -> HookObject {
    let handler = host.attach(|py| {
        let module =
            PyModule::from_code(py, HANDLER_CODE, c"b8043_method.py", c"b8043_method").unwrap();
        py.import("sys")
            .unwrap()
            .getattr("modules")
            .unwrap()
            .set_item("b8043_method", &module)
            .unwrap();
        module
            .getattr("Handler")
            .unwrap()
            .call1((10_i64,))
            .unwrap()
            .unbind()
    });
    HookObject {
        loaded: LoadedObject {
            object: handler,
            value: "b8043_method.Handler".to_owned(),
        },
    }
}

fn calls_recorded(host: &PythonHost) -> i64 {
    host.attach(|py| {
        py.import("b8043_method")
            .unwrap()
            .call_method0("recorded")
            .unwrap()
            .extract::<i64>()
            .unwrap()
    })
}

#[test]
fn has_method_finds_a_callable_attribute() {
    let host = start_host();
    let hook = handler_fixture(&host);
    assert!(hook.has_method(&host, "add"));
    assert!(hook.has_method(&host, "add_async"));
    assert!(!hook.has_method(&host, "missing"));
    assert!(!hook.has_method(&host, "label"));
}

#[test]
fn has_method_does_not_call_the_method() {
    let host = start_host();
    let hook = handler_fixture(&host);
    assert!(hook.has_method(&host, "touch"));
    assert_eq!(calls_recorded(&host), 0);
}

#[test]
fn method_returns_none_for_a_missing_method() {
    let host = start_host();
    let hook = handler_fixture(&host);
    assert!(hook.method(&host, "missing").is_none());
    assert!(hook.method(&host, "label").is_none());
}

#[tokio::test]
async fn method_returns_a_handle_to_the_bound_method() {
    let host = start_host();
    let hook = handler_fixture(&host);
    let add = hook.method(&host, "add").unwrap();
    let value = add
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![JsonValue::from(4_i64)],
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(value, JsonValue::from(14_i64));
}

#[tokio::test]
async fn call_method_calls_a_sync_method() {
    let host = start_host();
    let hook = handler_fixture(&host);
    let value = hook
        .call_method(
            &host,
            &tracker(),
            Duration::from_secs(5),
            MethodCall {
                name: "add",
                args: vec![JsonValue::from(5_i64)],
                kwargs: JsonObject::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(value, JsonValue::from(15_i64));
}

#[tokio::test]
async fn call_method_awaits_an_async_method() {
    let host = start_host();
    let hook = handler_fixture(&host);
    let value = hook
        .call_method(
            &host,
            &tracker(),
            Duration::from_secs(5),
            MethodCall {
                name: "add_async",
                args: vec![JsonValue::from(7_i64)],
                kwargs: JsonObject::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(value, JsonValue::from(17_i64));
}

#[tokio::test]
async fn call_method_passes_keyword_arguments_to_a_method() {
    let host = start_host();
    let hook = handler_fixture(&host);
    let value = hook
        .call_method(
            &host,
            &tracker(),
            Duration::from_secs(5),
            MethodCall {
                name: "add",
                args: Vec::new(),
                kwargs: JsonObject::from_iter([("value".to_owned(), JsonValue::from(6_i64))]),
            },
        )
        .await
        .unwrap();
    assert_eq!(value, JsonValue::from(16_i64));
}

#[tokio::test]
async fn call_method_reports_a_missing_method_as_no_such_method() {
    let host = start_host();
    let hook = handler_fixture(&host);
    let error = hook
        .call_method(
            &host,
            &tracker(),
            Duration::from_secs(5),
            MethodCall {
                name: "missing",
                args: Vec::new(),
                kwargs: JsonObject::new(),
            },
        )
        .await
        .unwrap_err();
    let HookError::NoSuchMethod { name } = error else {
        panic!("expected NoSuchMethod, got {error:?}");
    };
    assert_eq!(name, "missing");
}
