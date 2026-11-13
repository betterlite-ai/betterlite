#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::time::Duration;

use pyo3::types::{PyAnyMethods as _, PyModule};
use pyo3::{Py, PyAny};

use crate::blocking_work::WorkTracker;
use crate::hook_object::{HookError, HookObject, RawMethodCall};
use crate::host::{HostConfig, PythonHost};
use crate::instance_loader::LoadedObject;
use crate::json_values::{JsonObject, JsonValue};

const CONSTRUCT_CODE: &CStr = c"class Request:\n    def __init__(self, model, messages):\n        self.model = model\n        self.messages = messages\n\nclass Helper:\n    def count(self, request):\n        return len(request.messages)\n\nclass Defaults:\n    def __init__(self, value=5):\n        self.value = value\n\ndef factory():\n    return 'made'\n";

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn tracker() -> WorkTracker {
    WorkTracker::new(2, Duration::from_secs(30))
}

fn registered_module(host: &PythonHost) -> Py<PyAny> {
    host.attach(|py| {
        let module = PyModule::from_code(
            py,
            CONSTRUCT_CODE,
            c"b8043_construct.py",
            c"b8043_construct",
        )
        .unwrap();
        py.import("sys")
            .unwrap()
            .getattr("modules")
            .unwrap()
            .set_item("b8043_construct", &module)
            .unwrap();
        module.into_any().unbind()
    })
}

fn request_arguments() -> JsonValue {
    JsonValue::Object(JsonObject::from_iter([
        ("model".to_owned(), JsonValue::from("gpt-4")),
        (
            "messages".to_owned(),
            JsonValue::Array(vec![
                JsonValue::from("one"),
                JsonValue::from("two"),
                JsonValue::from("three"),
            ]),
        ),
    ]))
}

#[test]
fn construct_builds_a_class_from_a_json_object() {
    let host = start_host();
    registered_module(&host);
    let instance =
        HookObject::construct(&host, "b8043_construct.Request", &request_arguments()).unwrap();
    let (model, count) = host.attach(|py| {
        let bound = instance.bind(py);
        let model = bound.getattr("model").unwrap().extract::<String>().unwrap();
        let count = bound.getattr("messages").unwrap().len().unwrap();
        (model, count)
    });
    assert_eq!(model, "gpt-4");
    assert_eq!(count, 3);
}

#[test]
fn construct_uses_the_class_defaults_for_an_empty_object() {
    let host = start_host();
    registered_module(&host);
    let arguments = JsonValue::Object(JsonObject::new());
    let instance = HookObject::construct(&host, "b8043_construct.Defaults", &arguments).unwrap();
    let value = host.attach(|py| {
        instance
            .bind(py)
            .getattr("value")
            .unwrap()
            .extract::<i64>()
            .unwrap()
    });
    assert_eq!(value, 5);
}

#[test]
fn construct_refuses_arguments_that_are_not_a_json_object() {
    let host = start_host();
    registered_module(&host);
    let error = HookObject::construct(&host, "b8043_construct.Request", &JsonValue::from("text"))
        .unwrap_err();
    let HookError::ArgumentsNotObject { path } = error else {
        panic!("expected ArgumentsNotObject, got {error:?}");
    };
    assert_eq!(path, "b8043_construct.Request");
}

#[test]
fn construct_reports_a_missing_class() {
    let host = start_host();
    registered_module(&host);
    let arguments = JsonValue::Object(JsonObject::new());
    let error = HookObject::construct(&host, "b8043_construct.Missing", &arguments).unwrap_err();
    let HookError::Python(failure) = error else {
        panic!("expected a Python error, got {error:?}");
    };
    assert_eq!(failure.type_name, "AttributeError");
}

#[test]
fn construct_refuses_an_attribute_that_is_not_a_class() {
    let host = start_host();
    registered_module(&host);
    let arguments = JsonValue::Object(JsonObject::new());
    let error = HookObject::construct(&host, "b8043_construct.factory", &arguments).unwrap_err();
    let HookError::NotClass { path } = error else {
        panic!("expected NotClass, got {error:?}");
    };
    assert_eq!(path, "b8043_construct.factory");
}

#[tokio::test]
async fn call_method_raw_passes_python_objects_through() {
    let host = start_host();
    let module = registered_module(&host);
    let request =
        HookObject::construct(&host, "b8043_construct.Request", &request_arguments()).unwrap();
    let helper = host.attach(|py| {
        module
            .bind(py)
            .getattr("Helper")
            .unwrap()
            .call0()
            .unwrap()
            .unbind()
    });
    let hook = HookObject {
        loaded: LoadedObject {
            object: helper,
            value: "b8043_construct.Helper".to_owned(),
        },
    };
    let count = hook
        .call_method_raw(
            &host,
            &tracker(),
            Duration::from_secs(5),
            RawMethodCall {
                name: "count",
                args: vec![request],
                kwargs: BTreeMap::new(),
            },
        )
        .await
        .unwrap();
    let counted = host.attach(|py| count.bind(py).extract::<i64>().unwrap());
    assert_eq!(counted, 3);
}
