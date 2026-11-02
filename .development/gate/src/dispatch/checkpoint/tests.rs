#![cfg(test)]

use std::fs;

use super::{recover, session};

#[test]
fn an_interrupted_codex_event_stream_recovers_the_exact_task_session() {
    let folder = tempfile::tempdir().unwrap();
    let output = folder.path().join("attempt-3-worker.json");
    fs::write(
        output.with_extension("started.json"),
        r#"{"runner":"codex-worker","model":"test-model","session":null}"#,
    )
    .unwrap();
    fs::write(output.with_extension("stdout"), "{\"type\":\"thread.started\",\"thread_id\":\"task-a\"}\n{\"type\":\"item.started\"}\n{\"partial\":").unwrap();
    let saved = recover(&output).unwrap().unwrap();
    assert_eq!(saved.id, "task-a");
    assert_eq!(saved.runner, "codex-worker");
    assert_eq!(saved.model, "test-model");
    assert!(!output.exists());
}

#[test]
fn a_claude_timeout_preserves_the_preallocated_session() {
    let folder = tempfile::tempdir().unwrap();
    let output = folder.path().join("attempt-4-worker.json");
    fs::write(
        output.with_extension("started.json"),
        r#"{"runner":"claude-worker","model":"sonnet","session":"task-b"}"#,
    )
    .unwrap();
    fs::write(output.with_extension("stdout"), "").unwrap();
    assert_eq!(session(&output).unwrap().as_deref(), Some("task-b"));
}
