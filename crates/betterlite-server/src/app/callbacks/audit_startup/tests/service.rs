#![cfg(test)]

use std::fs;

use super::support::{
    Workspace, environment, flush, install, installed_names, load, record, start,
};
use crate::python_hooks::PythonHooks;

#[tokio::test]
async fn the_installed_python_object_receives_dispatched_events() {
    let workspace = Workspace::new();
    let module = workspace.recorder();
    let source = format!(
        "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - {module}.recorder\n"
    );
    let loaded = load(&source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    install(&fixture, &hooks, &loaded)
        .await
        .expect("the python callback installs");
    fixture.service.create_audit_log_for_update(record()).await;
    flush(&fixture.service).await;
    let delivered = fs::read_to_string(workspace.directory.path().join("delivered.txt"))
        .expect("the python callback wrote the payload it received");
    assert_eq!(delivered.trim(), "audit-1");
    assert_eq!(
        fixture.repository.inserted.lock().unwrap().len(),
        1,
        "the database write follows the callback"
    );
    hooks.shutdown().await;
}

#[tokio::test]
async fn a_named_callback_resolves_only_at_dispatch() {
    let source = "model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - otel\n";
    let loaded = load(source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(None);
    install(&fixture, &hooks, &loaded)
        .await
        .expect("the named callback installs");
    assert!(
        fixture.resolver.calls.lock().unwrap().is_empty(),
        "a dotless entry is never resolved at startup"
    );
    fixture.service.create_audit_log_for_update(record()).await;
    flush(&fixture.service).await;
    assert_eq!(
        *fixture.resolver.calls.lock().unwrap(),
        vec!["otel".to_owned()],
        "the dispatcher resolves the name on the first event"
    );
    assert_eq!(
        fixture.logger.delivered.lock().unwrap().len(),
        1,
        "the named callback receives the payload"
    );
}

#[tokio::test]
async fn a_loaded_class_is_ignored_and_the_write_continues() {
    let workspace = Workspace::new();
    let module = workspace.recorder();
    let entry = format!("{module}.Recorder");
    let source =
        format!("model_list: []\nlitellm_settings:\n  audit_log_callbacks:\n    - {entry}\n");
    let loaded = load(&source, &environment(&[]));
    let fixture = start(&loaded.settings);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    install(&fixture, &hooks, &loaded)
        .await
        .expect("an audit callback class loads without the callbacks-list validation");
    assert_eq!(installed_names(&fixture.service), vec![entry]);
    fixture.service.create_audit_log_for_update(record()).await;
    flush(&fixture.service).await;
    assert_eq!(
        fixture.repository.inserted.lock().unwrap().len(),
        1,
        "the database write still runs"
    );
    assert!(
        !workspace.directory.path().join("delivered.txt").exists(),
        "a class never runs as an audit handler"
    );
    assert!(
        fixture.resolver.calls.lock().unwrap().is_empty(),
        "a loaded class never reaches the named resolver"
    );
    hooks.shutdown().await;
}
