#![cfg(test)]

use super::support::{
    BROKEN_MODULE, Workspace, capture, environment, install, installed_names, load, start,
};
use crate::app::callbacks::audit_startup::AuditStartupError;
use crate::python_hooks::PythonHooks;

const WARNING: &str = "'audit_log_callbacks' is configured but audit logging is not enabled. Audit log callbacks will not fire.";

#[tokio::test]
async fn a_false_setting_warns_with_the_exact_litellm_text() {
    let source = "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - otel\n  store_audit_logs: false\n";
    let loaded = load(source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(None);
    let (captured, guard) = capture();
    install(&fixture, &hooks, &loaded)
        .await
        .expect("the callbacks install");
    drop(guard);
    let output = captured.text();
    assert!(output.contains(WARNING), "{output}");
    assert!(
        output.contains("WARN"),
        "the diagnostic is a warning: {output}"
    );
    assert_eq!(
        installed_names(&fixture.service).len(),
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
    install(&fixture, &hooks, &loaded)
        .await
        .expect("an empty list installs nothing");
    drop(guard);
    let output = captured.text();
    assert!(output.contains(WARNING), "{output}");
    assert!(
        installed_names(&fixture.service).is_empty(),
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
    let lookup = environment(&[("LITELLM_STORE_AUDIT_LOGS", "false")]);
    let loaded = load(&source, &lookup);
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let (captured, guard) = capture();
    install(&fixture, &hooks, &loaded)
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
    install(&fixture, &hooks, &loaded)
        .await
        .expect("a disabled policy does not suppress the import");
    drop(guard);
    assert_eq!(installed_names(&fixture.service), vec![entry]);
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
    let (captured, guard) = capture();
    let error = install(&fixture, &hooks, &loaded)
        .await
        .expect_err("a disabled policy cannot make a broken handler valid");
    drop(guard);
    assert!(!captured.text().contains(WARNING));
    match error {
        AuditStartupError::Load { entry, .. } => assert_eq!(entry, "b8872_broken.handler"),
    }
    assert!(
        installed_names(&fixture.service).is_empty(),
        "the failing entry leaves no callback behind"
    );
    hooks.shutdown().await;
}
