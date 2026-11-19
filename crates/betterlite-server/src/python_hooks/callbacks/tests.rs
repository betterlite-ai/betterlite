#![cfg(test)]

use rstest::rstest;

use super::super::HooksError;
use super::super::slots::{HookSlot, ListKey, LoadedCallback};
use super::super::test_support;

const CALLBACKS: &str = "class Logger:\n    pass\n\nnot_callable = 3\n\n\
def handler(**kwargs):\n    return \"ran\"\n";

#[rstest]
#[case::callbacks(ListKey::Callbacks)]
#[case::success_callback(ListKey::SuccessCallback)]
#[case::failure_callback(ListKey::FailureCallback)]
#[case::audit_log_callbacks(ListKey::AuditLogCallbacks)]
#[tokio::test]
async fn a_dotless_entry_stays_a_name(#[case] key: ListKey) {
    let service = test_support::service(None);
    let loaded = service.load_callback(key, "langfuse").await.unwrap();
    match loaded {
        LoadedCallback::Named(name) => assert_eq!(name, "langfuse"),
        LoadedCallback::Loaded(_hook) => panic!("a dotless name must never be loaded"),
    }
    assert!(
        service.inner.get().is_none(),
        "a dotless name must never start Python"
    );
}

#[rstest]
#[case::callbacks(ListKey::Callbacks)]
#[case::success_callback(ListKey::SuccessCallback)]
#[case::failure_callback(ListKey::FailureCallback)]
#[case::audit_log_callbacks(ListKey::AuditLogCallbacks)]
#[tokio::test]
async fn a_dotted_entry_is_loaded_for_every_list(#[case] key: ListKey) {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_callback_dotted", CALLBACKS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let entry = format!("{module}.handler");
    match service.load_callback(key, &entry).await.unwrap() {
        LoadedCallback::Loaded(hook) => {
            assert_eq!(hook.slot, HookSlot::Callback(key));
            assert_eq!(hook.value, entry);
        }
        LoadedCallback::Named(name) => panic!("a dotted entry must load, got the name {name}"),
    }
}

#[tokio::test]
async fn a_callbacks_entry_that_resolves_to_a_class_fails_with_litellms_message() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_callback_class", CALLBACKS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let entry = format!("{module}.Logger");
    let error = match service.load_callback(ListKey::Callbacks, &entry).await {
        Ok(_loaded) => panic!("a class must not load as a callback"),
        Err(error) => error,
    };
    assert!(
        matches!(error, HooksError::CallbackIsClass { .. }),
        "got: {error:?}"
    );
    let expected = format!(
        "litellm_settings.callbacks entry '{entry}' resolved to the class {module}.Logger, \
which is neither a CustomLogger instance nor a callable, so the proxy would never run it. \
Point it at an instance instead, e.g. add `proxy_handler_instance = Logger()` to {module} \
and set `callbacks: [\"{module}.proxy_handler_instance\"]`."
    );
    assert_eq!(error.to_string(), expected);
}

#[tokio::test]
async fn a_callbacks_entry_that_is_not_callable_fails_with_litellms_message() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_callback_int", CALLBACKS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let entry = format!("{module}.not_callable");
    let error = match service.load_callback(ListKey::Callbacks, &entry).await {
        Ok(_loaded) => panic!("an int must not load as a callback"),
        Err(error) => error,
    };
    assert!(
        matches!(error, HooksError::CallbackNotDispatchable { .. }),
        "got: {error:?}"
    );
    let expected = format!(
        "litellm_settings.callbacks entry '{entry}' resolved to int 3, which is neither a \
CustomLogger instance nor a callable, so the proxy would never run it."
    );
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::success_callback(ListKey::SuccessCallback)]
#[case::failure_callback(ListKey::FailureCallback)]
#[case::audit_log_callbacks(ListKey::AuditLogCallbacks)]
#[tokio::test]
async fn the_other_lists_take_a_class_without_checking(#[case] key: ListKey) {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_callback_other", CALLBACKS);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let entry = format!("{module}.Logger");
    match service.load_callback(key, &entry).await {
        Ok(LoadedCallback::Loaded(hook)) => assert_eq!(hook.slot, HookSlot::Callback(key)),
        Ok(LoadedCallback::Named(name)) => panic!("a dotted entry must load, got the name {name}"),
        Err(error) => panic!("only callbacks checks the kind, got: {error}"),
    }
}

#[tokio::test]
async fn load_callback_keeps_the_python_error_for_a_missing_module() {
    let service = test_support::service(None);
    let error = match service
        .load_callback(ListKey::Callbacks, "b165_absent_module.thing")
        .await
    {
        Ok(_loaded) => panic!("a missing module must not load"),
        Err(error) => error,
    };
    match error {
        HooksError::Load { slot, value, .. } => {
            assert_eq!(slot, "callbacks");
            assert_eq!(value, "b165_absent_module.thing");
        }
        other @ (HooksError::Interpreter { .. }
        | HooksError::InterpreterTask { .. }
        | HooksError::InterpreterMissing
        | HooksError::CallbackIsClass { .. }
        | HooksError::CallbackNotDispatchable { .. }
        | HooksError::Validate { .. }
        | HooksError::Call(_)
        | HooksError::StartupSpec { .. }
        | HooksError::Startup { .. }) => panic!("expected a load error, got {other:?}"),
    }
}
