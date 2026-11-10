#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::PrivateFiles;
use rstest::rstest;
use tempfile::TempDir;

use super::{env_object, load_json_or_empty, write_claude_settings, write_target};
use crate::claude_settings::error::ClaudeSettingsError;
use crate::claude_settings::{ENV_KEY, SettingsDocument};

fn settings_from(text: &str) -> SettingsDocument {
    serde_json::from_str::<SettingsDocument>(text).unwrap()
}

#[test]
fn load_json_or_empty_returns_an_empty_document_for_a_missing_file() {
    let home = TempDir::new().unwrap();
    let missing = home.path().join("settings.json");
    assert_eq!(
        load_json_or_empty(&missing).unwrap(),
        SettingsDocument::new()
    );
}

#[rstest]
#[case::empty("")]
#[case::spaces("  \n\t\r")]
#[case::form_feed_and_vertical_tab("\u{0b}\u{0c}")]
fn load_json_or_empty_returns_an_empty_document_for_a_blank_file(#[case] content: &str) {
    let home = TempDir::new().unwrap();
    let path = home.path().join("settings.json");
    fs::write(&path, content).unwrap();
    assert_eq!(load_json_or_empty(&path).unwrap(), SettingsDocument::new());
}

#[test]
fn load_json_or_empty_reads_a_document() {
    let home = TempDir::new().unwrap();
    let path = home.path().join("settings.json");
    fs::write(&path, r#"{"model":"claude-opus","env":{"FOO":"bar"}}"#).unwrap();
    let document = load_json_or_empty(&path).unwrap();
    assert_eq!(
        document.get("model"),
        Some(&JsonValue::String("claude-opus".to_owned()))
    );
    let env = document
        .get(ENV_KEY)
        .and_then(JsonValue::as_object)
        .unwrap();
    assert_eq!(env.get("FOO").and_then(JsonValue::as_str), Some("bar"));
}

#[rstest]
#[case::array("[]")]
#[case::string("\"settings\"")]
#[case::number("42")]
#[case::boolean("true")]
#[case::null("null")]
#[case::invalid("{\"model\":")]
fn load_json_or_empty_rejects_a_non_object_document(#[case] content: &str) {
    let home = TempDir::new().unwrap();
    let path = home.path().join("settings.json");
    fs::write(&path, content).unwrap();
    let error = load_json_or_empty(&path).unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::NotObject { .. }));
    assert!(error.source().is_some());
}

#[test]
fn load_json_or_empty_reports_an_unreadable_file() {
    let home = TempDir::new().unwrap();
    let error = load_json_or_empty(home.path()).unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::Read { .. }));
    assert!(error.source().is_some());
}

#[tokio::test]
async fn write_claude_settings_writes_the_document_as_json() {
    let home = TempDir::new().unwrap();
    let path = home.path().join("nested").join("settings.json");
    let document = settings_from(r#"{"env":{"ANTHROPIC_BASE_URL":"https://proxy.example.com"}}"#);
    write_claude_settings(&PrivateFiles::default(), &path, &document)
        .await
        .unwrap();
    assert_eq!(load_json_or_empty(&path).unwrap(), document);
}

#[cfg(unix)]
#[tokio::test]
async fn write_claude_settings_writes_an_owner_only_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = TempDir::new().unwrap();
    let path = home.path().join("settings.json");
    write_claude_settings(&PrivateFiles::default(), &path, &SettingsDocument::new())
        .await
        .unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[cfg(unix)]
#[tokio::test]
async fn write_claude_settings_writes_through_a_symlink() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let target = home.path().join("dotfiles").join("settings.json");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "{}").unwrap();
    let link = home.path().join("settings.json");
    symlink(&target, &link).unwrap();
    let document = settings_from(r#"{"model":"claude-opus"}"#);
    write_claude_settings(&PrivateFiles::default(), &link, &document)
        .await
        .unwrap();
    assert!(link.is_symlink());
    assert_eq!(load_json_or_empty(&target).unwrap(), document);
    assert_eq!(load_json_or_empty(&link).unwrap(), document);
}

#[cfg(unix)]
#[tokio::test]
async fn write_claude_settings_creates_the_target_of_a_dangling_symlink() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let target = home.path().join("dotfiles").join("settings.json");
    let link = home.path().join("settings.json");
    symlink(&target, &link).unwrap();
    let document = settings_from(r#"{"model":"claude-opus"}"#);
    write_claude_settings(&PrivateFiles::default(), &link, &document)
        .await
        .unwrap();
    assert!(link.is_symlink());
    assert_eq!(load_json_or_empty(&target).unwrap(), document);
}

#[tokio::test]
async fn write_claude_settings_reports_a_failed_write() {
    let home = TempDir::new().unwrap();
    let blocker = home.path().join("blocker");
    fs::write(&blocker, "not a directory").unwrap();
    let path = blocker.join("settings.json");
    let error = write_claude_settings(&PrivateFiles::default(), &path, &SettingsDocument::new())
        .await
        .unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::Write { .. }));
    assert!(error.source().is_some());
}

#[test]
fn write_target_keeps_a_plain_path() {
    let path = Path::new("/home/tester/.claude/settings.json");
    assert_eq!(
        write_target(path).unwrap(),
        PathBuf::from("/home/tester/.claude/settings.json")
    );
}

#[cfg(unix)]
#[test]
fn write_target_resolves_a_symlink() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let target = home.path().join("dotfiles").join("settings.json");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "{}").unwrap();
    let link = home.path().join("settings.json");
    symlink(&target, &link).unwrap();
    assert_eq!(
        write_target(&link).unwrap(),
        fs::canonicalize(&target).unwrap()
    );
}

#[test]
fn env_object_returns_an_empty_document_for_an_absent_env() {
    let settings = SettingsDocument::new();
    let env = env_object(&settings, Path::new("/settings.json")).unwrap();
    assert_eq!(env, SettingsDocument::new());
}

#[rstest]
#[case::null(r#"{"env":null}"#)]
#[case::no_env(r#"{"model":"claude-opus"}"#)]
fn env_object_treats_a_missing_or_null_env_as_absent(#[case] content: &str) {
    let settings = settings_from(content);
    let env = env_object(&settings, Path::new("/settings.json")).unwrap();
    assert!(env.is_empty());
}

#[test]
fn env_object_returns_the_env_entries() {
    let settings = settings_from(r#"{"env":{"FOO":"bar","COUNT":2}}"#);
    let env = env_object(&settings, Path::new("/settings.json")).unwrap();
    assert_eq!(env.get("FOO"), Some(&JsonValue::String("bar".to_owned())));
    assert_eq!(env.len(), 2);
}

#[rstest]
#[case::string(r#"{"env":"oops"}"#)]
#[case::array(r#"{"env":["oops"]}"#)]
#[case::number(r#"{"env":7}"#)]
#[case::boolean(r#"{"env":true}"#)]
fn env_object_rejects_a_non_object_env(#[case] content: &str) {
    let settings = settings_from(content);
    let error = env_object(&settings, Path::new("/settings.json")).unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::NonObjectEnv { .. }));
    let message = error.to_string();
    assert!(message.contains("env"));
    assert!(message.contains("/settings.json"));
}
