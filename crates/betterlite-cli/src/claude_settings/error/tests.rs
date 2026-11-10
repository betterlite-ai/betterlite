#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::io;
use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;
use tempfile::TempDir;

use crate::claude_settings::SettingsDocument;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::claude_settings::io::{
    env_object, load_json_or_empty, write_claude_settings, write_target,
};
use crate::claude_settings::paths::{
    claude_settings_paths, refuse_while_owned, settings_file_owners,
};
use crate::claude_settings::statusline_install::statusline_command;

fn source_is_kept(error: &ClaudeSettingsError) -> bool {
    error.source().is_some()
}

#[test]
fn an_unreadable_settings_file_keeps_its_cause() {
    let home = TempDir::new().unwrap();
    let error = load_json_or_empty(home.path()).unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::Read { .. }));
    assert!(source_is_kept(&error));
    assert!(
        error
            .to_string()
            .contains(&home.path().display().to_string())
    );
}

#[test]
fn a_non_object_settings_file_keeps_the_parser_cause() {
    let home = TempDir::new().unwrap();
    let path = home.path().join("settings.json");
    fs::write(&path, "[1, 2]").unwrap();
    let error = load_json_or_empty(&path).unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::NotObject { .. }));
    assert!(source_is_kept(&error));
    assert!(error.to_string().contains("settings.json"));
}

#[tokio::test]
async fn a_failed_settings_write_keeps_its_cause() {
    let home = TempDir::new().unwrap();
    let blocker = home.path().join("blocker");
    fs::write(&blocker, "not a directory").unwrap();
    let path = blocker.join("settings.json");
    let target = write_target(&path).unwrap();
    let error = write_claude_settings(&PrivateFiles::default(), &target, &SettingsDocument::new())
        .await
        .unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::Write { .. }));
    assert!(source_is_kept(&error));
}

#[test]
fn a_missing_executable_keeps_its_cause() {
    let error = ClaudeSettingsError::CurrentExe {
        source: io::Error::from(io::ErrorKind::NotFound),
    };
    assert!(source_is_kept(&error));
    assert!(error.to_string().contains("executable"));
}

#[test]
fn a_failed_quote_keeps_its_cause() {
    let error = statusline_command(Path::new("/opt/a\0b"), "linux").unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::Quote { .. }));
    assert!(source_is_kept(&error));
}

#[test]
fn a_non_object_env_names_the_settings_path() {
    let home = TempDir::new().unwrap();
    let path = home.path().join("settings.json");
    fs::write(&path, r#"{"env":"oops"}"#).unwrap();
    let settings = load_json_or_empty(&path).unwrap();
    let error = env_object(&settings, &path).unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::NonObjectEnv { .. }));
    assert!(error.to_string().contains("settings.json"));
    assert!(!source_is_kept(&error));
}

#[test]
fn an_ownership_refusal_names_the_start_and_stop_commands() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.backup.parent().unwrap()).unwrap();
    fs::write(&paths.backup, "{}").unwrap();
    let owners = settings_file_owners(&paths.settings, &paths).unwrap();
    let error = refuse_while_owned(&paths.settings, &owners).unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::Owned { .. }));
    let message = error.to_string();
    assert!(message.contains("lite up"));
    assert!(message.contains("lite down"));
    assert!(!source_is_kept(&error));
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_is_refused_without_a_cause() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let path = Path::new(OsStr::from_bytes(b"/opt/\xff/lite"));
    let error = statusline_command(path, "linux").unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::NonUtf8Path { .. }));
    assert!(!source_is_kept(&error));
}
