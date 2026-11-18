#![cfg(test)]

use std::time::Duration;

use betterlite_pyhost::blocking_work::WorkError;
use betterlite_pyhost::call_error::CallError;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};
use pyo3::types::PyAnyMethods as _;

use super::{HOOK_DEADLINE, LOAD_DEADLINE};
use crate::python_hooks::PythonHooks;
use crate::python_hooks::slots::HookSlot;
use crate::python_hooks::test_support;

const GATED: &str = "import threading\n\n_gate = threading.Event()\n\n\
def blocked():\n    _gate.wait()\n    return \"done\"\n\n\
def release():\n    _gate.set()\n";

fn release(service: &PythonHooks, module: &str) {
    let runtime = service.inner.get().unwrap();
    runtime.host.attach(|py| {
        py.import(module).unwrap().call_method0("release").unwrap();
    });
}

#[test]
fn the_default_hook_deadline_is_thirty_seconds() {
    assert_eq!(HOOK_DEADLINE, Duration::from_secs(30));
}

#[test]
fn the_load_deadline_is_sixty_seconds() {
    assert_eq!(LOAD_DEADLINE, Duration::from_secs(60));
}

#[tokio::test]
async fn call_within_returns_the_value_when_the_hook_finishes_in_time() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_call_within", "def quick():\n    return \"done\"\n");
    let service = test_support::service(Some(workspace.config_path.clone()));
    let hook = service
        .load_hook(HookSlot::CustomAuth, &format!("{module}.quick"))
        .await
        .unwrap();
    let deadline = Duration::from_secs(5);
    let result = hook
        .call_within(deadline, Vec::new(), JsonObject::new())
        .await
        .unwrap();
    assert_eq!(result, JsonValue::from("done"));
}

#[tokio::test]
async fn a_hook_that_passes_its_deadline_gives_a_timeout_error() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_hook_deadline", GATED);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let hook = service
        .load_hook(HookSlot::CustomAuth, &format!("{module}.blocked"))
        .await
        .unwrap();
    let deadline = Duration::from_millis(200);
    let error = hook
        .call_within(deadline, Vec::new(), JsonObject::new())
        .await
        .unwrap_err();
    release(&service, &module);
    match error {
        CallError::Work(WorkError::TimedOut { after }) => assert_eq!(after, deadline),
        other => panic!("expected the pool timeout, got {other:?}"),
    }
}
