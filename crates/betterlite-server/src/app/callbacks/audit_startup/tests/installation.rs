#![cfg(test)]

use super::support::{BROKEN_MODULE, Workspace, entry_name, environment, load, start};
use crate::app::callbacks::audit_startup::{AuditStartupError, install_audit_callbacks};
use crate::python_hooks::PythonHooks;
use crate::python_hooks::slots::LoadedCallback;

#[tokio::test]
async fn an_absent_list_keeps_the_installed_callbacks_and_the_cache() {
    let loaded = load("model_list: []\n", &environment(&[]));
    let fixture = start(&loaded.settings);
    fixture
        .service
        .append_audit_log_callback(LoadedCallback::Named("otel".to_owned()));
    let warm = fixture.service.delivery.named.resolve("otel", None);
    assert!(
        warm.is_some(),
        "the named callback resolves before the reload"
    );
    assert_eq!(fixture.resolver.calls.lock().unwrap().len(), 1);

    let hooks = PythonHooks::new(None);
    install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect("an absent key installs nothing");

    assert_eq!(
        fixture.service.callbacks.lock().unwrap().len(),
        1,
        "the installed callbacks survive an absent key"
    );
    let resolved = fixture.service.delivery.named.resolve("otel", None);
    assert!(
        resolved.is_some(),
        "the cached named callback survives an absent key"
    );
    assert_eq!(
        fixture.resolver.calls.lock().unwrap().len(),
        1,
        "an absent key does not reset the name cache"
    );
}

#[tokio::test]
async fn a_present_list_replaces_the_callbacks_in_order() {
    let workspace = Workspace::new();
    let module = workspace.recorder();
    let entry = format!("{module}.recorder");
    let source = format!(
        "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - otel\n    - {entry}\n"
    );
    let loaded = load(&source, &environment(&[]));
    let fixture = start(&loaded.settings);
    fixture
        .service
        .append_audit_log_callback(LoadedCallback::Named("stale".to_owned()));
    let warm = fixture.service.delivery.named.resolve("otel", None);
    assert!(
        warm.is_some(),
        "the named callback resolves before the reload"
    );
    assert_eq!(fixture.resolver.calls.lock().unwrap().len(), 1);

    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect("the callbacks install");

    let names: Vec<String> = fixture
        .service
        .callbacks
        .lock()
        .unwrap()
        .iter()
        .map(entry_name)
        .collect();
    assert_eq!(
        names,
        vec!["otel".to_owned(), entry],
        "the configured list replaces the old entries and keeps its order"
    );
    let resolved = fixture.service.delivery.named.resolve("otel", None);
    assert!(
        resolved.is_some(),
        "the named callback still resolves after the reload"
    );
    assert_eq!(
        fixture.resolver.calls.lock().unwrap().len(),
        2,
        "a present list resets the name cache"
    );
    hooks.shutdown().await;
}

#[tokio::test]
async fn a_failed_import_keeps_the_appended_prefix() {
    let workspace = Workspace::new();
    workspace.file("b8872_broken.py", BROKEN_MODULE);
    let module = workspace.recorder();
    let source = format!(
        "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - otel\n    - b8872_broken.handler\n    - {module}.recorder\n"
    );
    let loaded = load(&source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let warm = fixture.service.delivery.named.resolve("otel", None);
    assert!(
        warm.is_some(),
        "the named callback resolves before the reload"
    );
    assert_eq!(fixture.resolver.calls.lock().unwrap().len(), 1);

    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let error = install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect_err("a broken handler stops the install");
    match error {
        AuditStartupError::Load { entry, .. } => assert_eq!(entry, "b8872_broken.handler"),
    }

    let names: Vec<String> = fixture
        .service
        .callbacks
        .lock()
        .unwrap()
        .iter()
        .map(entry_name)
        .collect();
    assert_eq!(
        names,
        vec!["otel".to_owned()],
        "only the entries before the failure stay installed"
    );
    let resolved = fixture.service.delivery.named.resolve("otel", None);
    assert!(
        resolved.is_some(),
        "the appended prefix still resolves after the failure"
    );
    assert_eq!(
        fixture.resolver.calls.lock().unwrap().len(),
        2,
        "the name cache resets before a failing import"
    );
    hooks.shutdown().await;
}

#[tokio::test]
async fn an_explicit_empty_list_clears_the_callbacks() {
    let source = "model_list: []\nlitellm_settings:\n  audit_log_callbacks: []\n";
    let loaded = load(source, &environment(&[]));
    assert_eq!(loaded.configured, Some(Vec::new()), "the key is present");
    let fixture = start(&loaded.settings);
    fixture
        .service
        .append_audit_log_callback(LoadedCallback::Named("otel".to_owned()));
    let warm = fixture.service.delivery.named.resolve("otel", None);
    assert!(
        warm.is_some(),
        "the named callback resolves before the reload"
    );
    assert_eq!(fixture.resolver.calls.lock().unwrap().len(), 1);

    let hooks = PythonHooks::new(None);
    install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect("an empty list installs nothing");

    assert!(
        fixture.service.callbacks.lock().unwrap().is_empty(),
        "an explicit empty list clears the installed callbacks"
    );
    let resolved = fixture.service.delivery.named.resolve("otel", None);
    assert!(
        resolved.is_some(),
        "the named callback resolves after the empty list"
    );
    assert_eq!(
        fixture.resolver.calls.lock().unwrap().len(),
        2,
        "an explicit empty list resets the name cache"
    );
}
