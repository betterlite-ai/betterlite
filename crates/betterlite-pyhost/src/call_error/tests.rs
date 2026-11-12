#![cfg(test)]

use std::ffi::CStr;
use std::time::Duration;

use pyo3::types::{PyAnyMethods as _, PyModule};
use pyo3::{Py, PyAny};

use super::{CallError, PythonError};
use crate::blocking_work::WorkTracker;
use crate::callable_handle::CallableHandle;
use crate::host::{HostConfig, PythonHost};
use crate::json_values::{JsonObject, JsonValue, ValueError};

const ERROR_CODE: &CStr = c"class BoomBase(Exception):\n    pass\n\nclass Boom(BoomBase):\n    def __str__(self):\n        return 'boom!'\n\ndef fail():\n    raise Boom('ignored')\n\ndef fail_value():\n    raise ValueError('nope')\n\ndef quit_now():\n    raise SystemExit(3)\n\nasync def quit_async():\n    raise SystemExit(4)\n\nasync def interrupt_async():\n    raise KeyboardInterrupt()\n\nasync def fail_async():\n    raise ValueError('async-nope')\n\ndef returns_bytes():\n    return b'raw'\n";

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn code_module(host: &PythonHost, name: &CStr, code: &CStr) -> Py<PyAny> {
    host.attach(|py| {
        PyModule::from_code(py, code, c"hook_errors.py", name)
            .unwrap()
            .into_any()
            .unbind()
    })
}

fn attribute(host: &PythonHost, module: &Py<PyAny>, name: &str) -> Py<PyAny> {
    host.attach(|py| module.bind(py).getattr(name).unwrap().unbind())
}

async fn outcome(
    host: &PythonHost,
    module: &Py<PyAny>,
    name: &str,
) -> Result<JsonValue, CallError> {
    let tracker = WorkTracker::new(2, Duration::from_secs(30));
    let hook = CallableHandle::new(host, attribute(host, module, name)).unwrap();
    hook.call(
        host,
        &tracker,
        Duration::from_secs(5),
        Vec::new(),
        JsonObject::new(),
    )
    .await
}

async fn python_error(host: &PythonHost, module: &Py<PyAny>, name: &str) -> PythonError {
    match outcome(host, module, name).await {
        Err(CallError::Python(error)) => error,
        other => panic!("expected a Python error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_raised_exception_carries_its_type_module_message_and_traceback() {
    let host = start_host();
    let module = code_module(&host, c"hook_errors", ERROR_CODE);
    let error = python_error(&host, &module, "fail").await;
    assert_eq!(error.type_name, "Boom");
    assert_eq!(error.module, "hook_errors");
    assert_eq!(error.message, "boom!");
    assert!(
        error
            .traceback
            .contains("Traceback (most recent call last)"),
        "traceback was: {}",
        error.traceback
    );
    assert!(
        error.traceback.contains("hook_errors.py"),
        "traceback was: {}",
        error.traceback
    );
    assert!(
        error.traceback.contains("Boom: boom!"),
        "traceback was: {}",
        error.traceback
    );
}

#[tokio::test]
async fn a_builtin_exception_carries_the_builtins_module() {
    let host = start_host();
    let module = code_module(&host, c"hook_errors", ERROR_CODE);
    let error = python_error(&host, &module, "fail_value").await;
    assert_eq!(error.type_name, "ValueError");
    assert_eq!(error.module, "builtins");
    assert_eq!(error.message, "nope");
}

#[tokio::test]
async fn system_exit_from_a_sync_hook_is_a_python_error_and_the_process_continues() {
    let host = start_host();
    let module = code_module(&host, c"hook_errors", ERROR_CODE);
    let error = python_error(&host, &module, "quit_now").await;
    assert_eq!(error.type_name, "SystemExit");
    assert_eq!(error.message, "3");
    let after = python_error(&host, &module, "fail_value").await;
    assert_eq!(after.type_name, "ValueError");
}

#[tokio::test]
async fn system_exit_from_an_async_hook_is_a_python_error_and_the_loop_keeps_serving() {
    let host = start_host();
    let module = code_module(&host, c"hook_errors", ERROR_CODE);
    let error = python_error(&host, &module, "quit_async").await;
    assert_eq!(error.type_name, "SystemExit");
    assert_eq!(error.message, "4");
    let after = python_error(&host, &module, "fail_value").await;
    assert_eq!(after.type_name, "ValueError");
}

#[tokio::test]
async fn keyboard_interrupt_from_an_async_hook_is_a_python_error() {
    let host = start_host();
    let module = code_module(&host, c"hook_errors", ERROR_CODE);
    let error = python_error(&host, &module, "interrupt_async").await;
    assert_eq!(error.type_name, "KeyboardInterrupt");
}

#[tokio::test]
async fn an_async_hook_that_raises_an_exception_is_a_python_error_not_a_timeout() {
    let host = start_host();
    let module = code_module(&host, c"hook_errors", ERROR_CODE);
    let error = python_error(&host, &module, "fail_async").await;
    assert_eq!(error.type_name, "ValueError");
    assert_eq!(error.message, "async-nope");
    assert!(
        error.traceback.contains("ValueError: async-nope"),
        "traceback was: {}",
        error.traceback
    );
}

#[tokio::test]
async fn is_instance_of_matches_the_raised_exception_and_its_bases() {
    let host = start_host();
    let module = code_module(&host, c"hook_errors", ERROR_CODE);
    let error = python_error(&host, &module, "fail").await;
    assert!(error.is_instance_of(&host, "hook_errors.Boom"));
    assert!(error.is_instance_of(&host, "hook_errors.BoomBase"));
    assert!(error.is_instance_of(&host, "builtins.Exception"));
    assert!(!error.is_instance_of(&host, "hook_errors.ValueError"));
    assert!(!error.is_instance_of(&host, "builtins.ValueError"));
    assert!(!error.is_instance_of(&host, "builtins.KeyError"));
    assert!(!error.is_instance_of(&host, "missing_module.Boom"));
    assert!(!error.is_instance_of(&host, "hook_errors"));
}

#[tokio::test]
async fn a_hook_result_without_a_json_form_is_a_value_error() {
    let host = start_host();
    let module = code_module(&host, c"hook_errors", ERROR_CODE);
    match outcome(&host, &module, "returns_bytes").await {
        Err(CallError::Value(ValueError::Unsupported { type_name })) => {
            assert_eq!(type_name, "bytes");
        }
        other => panic!("expected a value error, got {other:?}"),
    }
}
