#![cfg(test)]

use std::time::Duration;

use betterlite_pyhost::call_error::CallError;
use betterlite_pyhost::hook_object::HookObject;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};
use pyo3::types::PyAnyMethods as _;
use serde_json::json;

use super::CustomProviderItem;
use crate::python_hooks::HooksError;
use crate::python_hooks::HooksRuntime;
use crate::python_hooks::PythonHooks;
use crate::python_hooks::slots::HookSlot;
use crate::python_hooks::test_support;

const CALLS: &str = "def sync_auth(key):\n    return {\"key\": key}\n\n\
async def async_auth(key):\n    return {\"key\": key, \"async\": True}\n";

const STARTUP: &str = "ORDER = []\n\n\
def record_first():\n    ORDER.append(\"first\")\n\n\
async def record_second():\n    ORDER.append(\"second\")\n\n\
def failing():\n    ORDER.append(\"failed\")\n    raise RuntimeError(\"boom\")\n\n\
def record_after_failure():\n    ORDER.append(\"after\")\n";

const PROVIDERS: &str = "def first_handler(**kwargs):\n    return \"first handler\"\n\n\
def second_handler(**kwargs):\n    return \"second handler\"\n";

const VALIDATORS: &str = "from typing import Literal\n\n\
def annotated(data) -> Literal[True]:\n    return True\n\n\
def plain(data) -> bool:\n    return True\n";

fn recorded(service: &PythonHooks, module: &str) -> Vec<String> {
    let runtime = service.inner.get().unwrap();
    runtime.host.attach(|py| {
        py.import(module)
            .unwrap()
            .getattr("ORDER")
            .unwrap()
            .extract::<Vec<String>>()
            .unwrap()
    })
}

async fn call_provider(runtime: &HooksRuntime, provider: &(String, HookObject)) -> JsonValue {
    let handle = provider.1.loaded.handle(&runtime.host).unwrap();
    handle
        .call(
            &runtime.host,
            &runtime.tracker,
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap()
}

#[test]
fn building_the_service_starts_no_interpreter() {
    let service = test_support::service(None);
    assert!(service.inner.get().is_none());
}

#[tokio::test]
async fn the_first_load_starts_the_interpreter_and_keeps_the_value() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_first_load", CALLS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let value = format!("{module}.sync_auth");
    let hook = service
        .load_hook(HookSlot::CustomAuth, &value)
        .await
        .unwrap();
    assert!(service.inner.get().is_some());
    assert_eq!(hook.value, value);
    assert_eq!(hook.slot, HookSlot::CustomAuth);
}

#[tokio::test]
async fn a_sync_hook_next_to_the_config_file_runs() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_sync_call", CALLS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let hook = service
        .load_hook(HookSlot::CustomAuth, &format!("{module}.sync_auth"))
        .await
        .unwrap();
    let result = hook
        .call(vec![JsonValue::from("sk-1")], JsonObject::new())
        .await
        .unwrap();
    assert_eq!(result, json!({"key": "sk-1"}));
}

#[tokio::test]
async fn an_async_hook_next_to_the_config_file_is_awaited() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_async_call", CALLS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let hook = service
        .load_hook(HookSlot::CustomKeyUpdate, &format!("{module}.async_auth"))
        .await
        .unwrap();
    let result = hook
        .call(vec![JsonValue::from("sk-2")], JsonObject::new())
        .await
        .unwrap();
    assert_eq!(result, json!({"key": "sk-2", "async": true}));
}

#[tokio::test]
async fn two_first_loads_at_once_start_one_interpreter() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_two_loads", CALLS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let value = format!("{module}.sync_auth");
    let (first, second) = tokio::join!(
        service.load_hook(HookSlot::CustomAuth, &value),
        service.load_hook(HookSlot::CustomKeyUpdate, &value),
    );
    first.unwrap();
    second.unwrap();
    assert!(service.inner.get().is_some());
}

#[tokio::test]
async fn a_load_error_names_the_slot_and_the_value() {
    let workspace = test_support::workspace();
    let service = test_support::service(Some(workspace.config_path.clone()));
    let value = "b165_missing_module.thing";
    let error = match service.load_hook(HookSlot::CustomKeyGenerate, value).await {
        Ok(_hook) => panic!("a missing module must not load"),
        Err(error) => error,
    };
    match error {
        HooksError::Load {
            slot,
            value: named,
            source,
        } => {
            assert_eq!(slot, "custom_key_generate");
            assert_eq!(named, value);
            assert!(
                source.to_string().contains("b165_missing_module"),
                "got: {source}"
            );
        }
        other => panic!("expected a load error, got {other:?}"),
    }
}

#[tokio::test]
async fn post_call_rules_load_as_a_one_element_list() {
    let workspace = test_support::workspace();
    let module = workspace.module(
        "b165_post_rule",
        "def rule(response):\n    return response\n",
    );
    let service = test_support::service(Some(workspace.config_path.clone()));
    let value = format!("{module}.rule");
    let rules = service.load_post_call_rules(&value).await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].slot, HookSlot::PostCallRules);
    assert_eq!(rules[0].value, value);
}

#[tokio::test]
async fn custom_providers_keep_the_config_order_and_the_paired_handler() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_providers", PROVIDERS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let items = vec![
        CustomProviderItem {
            provider: "first".to_owned(),
            custom_handler: format!("{module}.second_handler"),
        },
        CustomProviderItem {
            provider: "second".to_owned(),
            custom_handler: format!("{module}.first_handler"),
        },
    ];
    let providers = service.load_custom_providers(&items).await.unwrap();
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[0].0, "first");
    assert_eq!(providers[1].0, "second");
    let runtime = service.inner.get().unwrap();
    assert_eq!(
        call_provider(runtime, &providers[0]).await,
        json!("second handler")
    );
    assert_eq!(
        call_provider(runtime, &providers[1]).await,
        json!("first handler")
    );
}

#[tokio::test]
async fn custom_validate_accepts_a_literal_true_annotation() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_validate_ok", VALIDATORS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let hook = service
        .load_custom_validate(&format!("{module}.annotated"))
        .await
        .unwrap();
    assert_eq!(hook.slot, HookSlot::CustomValidate);
}

#[tokio::test]
async fn custom_validate_refuses_a_function_without_literal_true() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_validate_plain", VALIDATORS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let value = format!("{module}.plain");
    let error = match service.load_custom_validate(&value).await {
        Ok(_hook) => panic!("a bool annotation must not pass"),
        Err(error) => error,
    };
    match error {
        HooksError::Validate {
            value: named,
            source,
        } => {
            assert_eq!(named, value);
            let text = source.to_string();
            assert!(
                text.contains(
                    "Custom validator must be annotated to return Literal[True], got <class 'bool'>"
                ),
                "got: {text}"
            );
        }
        other => panic!("expected a validate error, got {other:?}"),
    }
}

#[tokio::test]
async fn startup_hooks_run_in_order_and_await_coroutines() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_startup_order", STARTUP);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let (captured, _guard) = test_support::capture();
    let specs = vec![
        format!("{module}.record_first"),
        format!("{module}.record_second"),
    ];
    service.run_startup_hooks(&specs).await.unwrap();
    assert_eq!(
        recorded(&service, &module),
        vec!["first".to_owned(), "second".to_owned()]
    );
    let logs = captured.text();
    let first = format!("Worker startup hook '{}' executed successfully", specs[0]);
    let second = format!("Worker startup hook '{}' executed successfully", specs[1]);
    assert!(logs.contains(&first), "logs were: {logs}");
    assert!(logs.contains(&second), "logs were: {logs}");
}

#[tokio::test]
async fn a_spec_without_a_colon_is_rejected_with_litellms_message() {
    let service = test_support::service(None);
    let specs = vec!["no_colon".to_owned()];
    let error = service.run_startup_hooks(&specs).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid hook spec 'no_colon': expected format is 'module.path:function_name'"
    );
}

#[tokio::test]
async fn a_failing_hook_reports_litellms_line_and_stops_the_later_specs() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_startup_failure", STARTUP);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let (captured, _guard) = test_support::capture();
    let specs = vec![
        format!("{module}.failing"),
        format!("{module}.record_after_failure"),
    ];
    let error = service.run_startup_hooks(&specs).await.unwrap_err();
    match error {
        HooksError::Startup {
            spec,
            error: message,
        } => {
            assert_eq!(spec, specs[0]);
            assert_eq!(message, "boom");
        }
        other => panic!("expected a startup error, got {other:?}"),
    }
    assert_eq!(recorded(&service, &module), vec!["failed".to_owned()]);
    let logs = captured.text();
    assert!(
        logs.contains(&format!("Worker startup hook '{}' failed: boom", specs[0])),
        "logs were: {logs}"
    );
    assert!(!logs.contains("executed successfully"), "logs were: {logs}");
}

#[tokio::test]
async fn a_startup_hook_that_cannot_be_imported_reports_the_python_message() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_startup_import", "def present():\n    return None\n");
    let service = test_support::service(Some(workspace.config_path.clone()));
    let specs = vec![format!("{module}.absent")];
    let error = service.run_startup_hooks(&specs).await.unwrap_err();
    match error {
        HooksError::Startup {
            spec,
            error: message,
        } => {
            assert_eq!(spec, specs[0]);
            assert!(message.contains("absent"), "got: {message}");
        }
        other => panic!("expected a startup error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_hook_that_raises_passes_the_python_error_to_the_caller() {
    let workspace = test_support::workspace();
    let module = workspace.module(
        "b165_hook_raises",
        "def broken(key):\n    raise ValueError(\"nope\")\n",
    );
    let service = test_support::service(Some(workspace.config_path.clone()));
    let hook = service
        .load_hook(HookSlot::CustomSso, &format!("{module}.broken"))
        .await
        .unwrap();
    let error = hook
        .call(vec![JsonValue::from("sk-3")], JsonObject::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, CallError::Python(_)),
        "expected the Python error, got {error:?}"
    );
    assert!(error.to_string().contains("nope"), "got: {error}");
}
