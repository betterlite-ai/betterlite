use secrecy::SecretString;

use super::support::{World, record};
use crate::settings_backup::report::restore_and_report;
use crate::up::command::run_up;
use crate::up::error::UpError;

const FRESH: f64 = 4_000_000_000.0;

fn prepared(settings: Option<&str>) -> World {
    let mut world = World::new(false);
    world.seed_token(record("sk-live", 1_000.0, Some(FRESH)));
    world.context.api_key = Some(SecretString::from("sk-live".to_owned()));
    if let Some(text) = settings {
        world.seed(&world.settings_path(), text);
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
        super::support::json_file(&world.settings_path()),
        serde_json::json!({"model": "opus"})
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
    let mut world = prepared(Some(r#"{"model":"opus","statusLine":{"type":"command"}}"#));
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    let snapshot = world
        .snapshot()
        .expect("the stop hook saw the patched state");
    assert_eq!(
        snapshot.backup,
        serde_json::json!({
            "existed": true,
            "content": {"model": "opus", "statusLine": {"type": "command"}},
        })
    );
    let patched = snapshot.settings;
    assert_eq!(
        patched.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
        Some(&serde_json::json!("sk-live"))
    );
    assert_eq!(
        patched.pointer("/env/ANTHROPIC_BASE_URL"),
        Some(&serde_json::json!("http://gateway.test:4000"))
    );
    assert_eq!(
        patched.pointer("/statusLine/command"),
        Some(&serde_json::json!("/opt/betterlite statusline"))
    );
}

#[test]
fn up_reports_when_the_settings_directory_cannot_be_created() {
    let mut world = prepared(None);
    std::fs::write(world.home_path().join(".claude"), "not a directory")
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
        serde_json::json!({"existed": false, "content": null})
    );
}

#[test]
fn down_without_a_backup_reports_nothing_to_restore() {
    let world = prepared(None);
    let mut stdout = Vec::new();
    restore_and_report(world.home_path(), &world.private_files, &mut stdout)
        .expect("there is nothing to restore");
    assert_eq!(String::from_utf8_lossy(&stdout), "Nothing to restore.\n");
}

#[test]
fn down_reports_a_corrupted_backup_with_its_path() {
    let world = prepared(None);
    world.seed(&world.backup_path(), "{not json");
    let mut stdout = Vec::new();
    let error = restore_and_report(world.home_path(), &world.private_files, &mut stdout)
        .expect_err("the backup cannot be read");
    let path = world.backup_path().display();
    assert_eq!(
        error.to_string(),
        format!("{path} contains invalid or unexpected JSON; cannot restore from it safely.")
    );
}
