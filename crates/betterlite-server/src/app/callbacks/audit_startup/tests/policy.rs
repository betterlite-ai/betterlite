#![cfg(test)]

use super::support::{BROKEN_MODULE, Workspace, capture, entry_name, environment, load, start};
use crate::app::callbacks::audit_startup::{AuditStartupError, install_audit_callbacks};
use crate::python_hooks::PythonHooks;

const WARNING: &str = "'audit_log_callbacks' is configured but audit logging is not enabled. Audit log callbacks will not fire.";

#[tokio::test]
async fn a_false_setting_warns_with_the_exact_litellm_text() {
    let source = "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - otel\n  store_audit_logs: false\n";
    let loaded = load(source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(None);
    let (captured, guard) = capture();
    install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect("the callbacks install");
    drop(guard);
    let output = captured.text();
    assert!(output.contains(WARNING), "{output}");
    assert_eq!(
        fixture.service.callbacks.lock().unwrap().len(),
        1,
        "a disabled policy still installs the configured callbacks"
    );
}

#[tokio::test]
async fn an_explicit_empty_list_still_warns() {
    let source =
        "model_list: []\nlitellm_settings:\n  audit_log_callbacks: []\n  store_audit_logs: false\n";
    let loaded = load(source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(None);
    let (captured, guard) = capture();
    install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect("an empty list installs nothing");
    drop(guard);
    let output = captured.text();
    assert!(output.contains(WARNING), "{output}");
    assert!(
        fixture.service.callbacks.lock().unwrap().is_empty(),
        "an explicit empty list stays empty"
    );
}

#[tokio::test]
async fn a_true_setting_overrides_a_false_environment_value() {
    let workspace = Workspace::new();
    let module = workspace.recorder();
    let entry = format!("{module}.recorder");
    let source = format!(
        "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - {entry}\n  store_audit_logs: true\n"
    );
    let environment = environment(&[("LITELLM_STORE_AUDIT_LOGS", "false")]);
    let loaded = load(&source, &environment);
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let (captured, guard) = capture();
    install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect("the callback installs");
    drop(guard);
    let output = captured.text();
    assert!(!output.contains(WARNING), "{output}");
    assert!(
        output.contains("Initialized Audit Log Callbacks"),
        "an enabled policy reports the initialization: {output}"
    );
    assert!(
        output.contains(&entry),
        "the initialization names the configured callback: {output}"
    );
    assert!(
        !output.contains("object at"),
        "the initialization never prints the loaded object: {output}"
    );
    hooks.shutdown().await;
}

#[tokio::test]
async fn a_disabled_policy_still_imports_the_configured_handler() {
    let workspace = Workspace::new();
    let module = workspace.recorder();
    let entry = format!("{module}.recorder");
    let source = format!(
        "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - {entry}\n  store_audit_logs: false\n"
    );
    let loaded = load(&source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let (captured, guard) = capture();
    install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect("a disabled policy does not suppress the import");
    drop(guard);
    let names: Vec<String> = fixture
        .service
        .callbacks
        .lock()
        .unwrap()
        .iter()
        .map(entry_name)
        .collect();
    assert_eq!(names, vec![entry]);
    assert!(captured.text().contains(WARNING));
    hooks.shutdown().await;
}

#[tokio::test]
async fn a_broken_handler_fails_with_auditing_disabled() {
    let workspace = Workspace::new();
    workspace.file("b8872_broken.py", BROKEN_MODULE);
    let source = "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - b8872_broken.handler\n  store_audit_logs: false\n";
    let loaded = load(source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let error = install_audit_callbacks(
        &fixture.service,
        &hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
    .expect_err("a disabled policy cannot make a broken handler valid");
    match error {
        AuditStartupError::Load { entry, .. } => assert_eq!(entry, "b8872_broken.handler"),
    }
    assert!(
        fixture.service.callbacks.lock().unwrap().is_empty(),
        "the failing entry leaves no callback behind"
    );
    hooks.shutdown().await;
}
