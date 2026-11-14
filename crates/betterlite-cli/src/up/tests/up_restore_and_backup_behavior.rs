use std::fs;

use betterlite_config::sections::settings::JsonValue;
use secrecy::SecretString;

use super::support::{World, document, json_file, record, seed, utf8};
use crate::settings_backup::report::restore_and_report;
use crate::up::command::run_up;
use crate::up::error::UpError;

const FRESH: f64 = 4_000_000_000.0;

fn prepared(settings: Option<&str>) -> World {
    let mut world = World::new(false);
    world.seed_token(record("sk-live", 1_000.0, Some(FRESH)));
    world.context.api_key = Some(SecretString::from("sk-live".to_owned()));
    if let Some(text) = settings {
        seed(&world.settings_path(), text);
    }
    world
}

#[test]
fn up_restores_the_original_settings_after_stopping() {
    let mut world = prepared(Some(r#"{"model":"opus"}"#));
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    assert_eq!(
        json_file(&world.settings_path()),
        Some(document(r#"{"model":"opus"}"#))
    );
    assert!(!world.backup_path().exists());
    let text = world.text();
    assert!(
        text.contains("litellm: routing Claude Code through proxy at http://gateway.test:4000")
    );
    assert!(text.contains("Press Ctrl-C to stop and restore your original settings."));
    assert_eq!(text.matches("Restored ").count(), 1);
}

#[test]
fn up_removes_settings_it_created_after_stopping() {
    let mut world = prepared(None);
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    assert!(!world.settings_path().exists());
    assert!(!world.backup_path().exists());
    assert_eq!(world.text().matches("Removed ").count(), 1);
}

#[test]
fn up_records_the_original_settings_in_the_backup_before_patching() {
    let mut world = prepared(Some(r#"{"model":"opus","theme":"dark"}"#));
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    let snapshot = world
        .snapshot()
        .expect("the stop hook saw the patched state");
    assert_eq!(
        snapshot.backup,
        Some(document(
            r#"{"existed":true,"content":{"model":"opus","theme":"dark"}}"#
        ))
    );
    let patched = snapshot.settings.expect("the settings were patched");
    let env = patched.get("env").expect("the env block was written");
    assert_eq!(
        env.get("ANTHROPIC_AUTH_TOKEN").and_then(JsonValue::as_str),
        Some("sk-live")
    );
    assert_eq!(
        env.get("ANTHROPIC_BASE_URL").and_then(JsonValue::as_str),
        Some("http://gateway.test:4000")
    );
    assert_eq!(
        patched
            .get("statusLine")
            .and_then(|entry| entry.get("command"))
            .and_then(JsonValue::as_str),
        Some("/opt/betterlite statusline")
    );
    assert_eq!(
        patched.get("theme").and_then(JsonValue::as_str),
        Some("dark")
    );
}

#[test]
fn up_keeps_a_status_line_the_user_runs_themselves() {
    let mut world = prepared(Some(
        r#"{"statusLine":{"type":"command","command":"my-own-line"}}"#,
    ));
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    let patched = world
        .snapshot()
        .and_then(|snapshot| snapshot.settings)
        .expect("the settings were patched");
    assert_eq!(
        patched
            .get("statusLine")
            .and_then(|entry| entry.get("command"))
            .and_then(JsonValue::as_str),
        Some("my-own-line")
    );
}

#[test]
fn up_reports_when_the_settings_directory_cannot_be_created() {
    let mut world = prepared(None);
    fs::write(world.home_path().join(".claude"), "not a directory")
        .expect("a file where the settings directory goes");
    let mut ports = world.ports();
    let error = run_up(&mut ports).expect_err("the settings directory cannot be created");
    assert!(matches!(error, UpError::SettingsDirectory { .. }));
}

#[test]
fn up_remembers_that_the_settings_did_not_exist() {
    let mut world = prepared(None);
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    let snapshot = world
        .snapshot()
        .expect("the stop hook saw the patched state");
    assert_eq!(
        snapshot.backup,
        Some(document(r#"{"existed":false,"content":null}"#))
    );
}

#[test]
fn down_without_a_backup_reports_nothing_to_restore() {
    let world = prepared(None);
    let mut stdout = Vec::new();
    restore_and_report(world.home_path(), &world.quiet.private_files, &mut stdout)
        .expect("there is nothing to restore");
    assert_eq!(utf8(&stdout), "Nothing to restore.\n");
}

#[test]
fn down_reports_a_corrupted_backup_with_its_path() {
    let world = prepared(None);
    seed(&world.backup_path(), "{not json");
    let mut stdout = Vec::new();
    let error = restore_and_report(world.home_path(), &world.quiet.private_files, &mut stdout)
        .expect_err("the backup cannot be read");
    let backup_path = world.backup_path();
    let path = backup_path.display();
    assert_eq!(
        error.to_string(),
        format!("{path} contains invalid or unexpected JSON; cannot restore from it safely.")
    );
}
