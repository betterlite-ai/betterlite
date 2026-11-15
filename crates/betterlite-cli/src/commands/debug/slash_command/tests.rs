#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use super::{SLASH_COMMAND_NAME, install_slash_command};
use crate::commands::debug::error::DebugError;

const EXPECTED_BODY: &str = "---
description: Pull the LiteLLM debug report (spend, request, response, error) for this Claude Code session
allowed-tools: Bash(lite debug claude:*)
---
Below is the LiteLLM debug report for this Claude Code session. Summarize the failing
request(s) in a few sentences (model, error, request id) and tell me the path the full
report was saved to so I can hand it off. If nothing failed, say so.

!`lite debug claude $ARGUMENTS`
";

#[test]
fn install_slash_command_writes_the_command_file_under_commands() {
    let claude_dir = tempfile::tempdir().unwrap();

    let path = install_slash_command(claude_dir.path()).unwrap();

    assert_eq!(
        path,
        claude_dir.path().join("commands").join("debug-lite.md")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), EXPECTED_BODY);
}

#[test]
fn install_slash_command_creates_a_missing_claude_directory() {
    let home = tempfile::tempdir().unwrap();
    let claude_dir = home.path().join(".claude");

    let path = install_slash_command(&claude_dir).unwrap();

    assert!(path.is_file());
}

#[test]
fn install_slash_command_names_the_command_debug_lite() {
    assert_eq!(SLASH_COMMAND_NAME, "debug-lite");
}

#[test]
fn install_slash_command_replaces_an_older_command_file() {
    let claude_dir = tempfile::tempdir().unwrap();
    let commands = claude_dir.path().join("commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(commands.join("debug-lite.md"), "an older command with a much longer body than the new one has, so a partial overwrite would show").unwrap();

    let path = install_slash_command(claude_dir.path()).unwrap();

    assert_eq!(fs::read_to_string(path).unwrap(), EXPECTED_BODY);
}

#[test]
fn install_slash_command_leaves_other_commands_alone() {
    let claude_dir = tempfile::tempdir().unwrap();
    let commands = claude_dir.path().join("commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(commands.join("other.md"), "keep me").unwrap();

    install_slash_command(claude_dir.path()).unwrap();

    assert_eq!(
        fs::read_to_string(commands.join("other.md")).unwrap(),
        "keep me"
    );
}

#[test]
fn install_slash_command_uses_the_default_file_permissions() {
    let claude_dir = tempfile::tempdir().unwrap();
    let plain = claude_dir.path().join("plain");
    fs::File::create(&plain).unwrap();

    let path = install_slash_command(claude_dir.path()).unwrap();

    let installed = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let expected = fs::metadata(plain).unwrap().permissions().mode() & 0o777;
    assert_eq!(installed, expected);
}

#[test]
fn install_slash_command_reports_a_commands_path_that_is_a_file() {
    let claude_dir = tempfile::tempdir().unwrap();
    fs::write(claude_dir.path().join("commands"), "").unwrap();

    let error = install_slash_command(claude_dir.path()).unwrap_err();

    assert!(
        matches!(&error, DebugError::SlashCommandDirectory { path, .. } if path.ends_with("commands"))
    );
    assert!(error.source().is_some());
}

#[test]
fn install_slash_command_reports_a_command_path_that_is_a_directory() {
    let claude_dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(claude_dir.path().join("commands").join("debug-lite.md")).unwrap();

    let error = install_slash_command(claude_dir.path()).unwrap_err();

    assert!(
        matches!(&error, DebugError::SlashCommandWrite { path, .. } if path.ends_with("debug-lite.md"))
    );
    assert!(error.source().is_some());
}
