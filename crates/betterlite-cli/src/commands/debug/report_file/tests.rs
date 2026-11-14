#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use rstest::rstest;

use super::write_report;
use crate::commands::debug::error::DebugError;

fn mode_of(path: &std::path::Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn write_report_creates_the_directory_and_names_the_file_after_the_session() {
    let home = tempfile::tempdir().unwrap();
    let report_dir = home.path().join(".litellm").join("debug");

    let path = write_report("# report\n", "sess-1", &report_dir).unwrap();

    assert_eq!(path, report_dir.join("claude-sess-1.md"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "# report\n");
}

#[test]
fn write_report_accepts_an_existing_directory() {
    let report_dir = tempfile::tempdir().unwrap();

    let path = write_report("x", "s", report_dir.path()).unwrap();

    assert_eq!(fs::read_to_string(path).unwrap(), "x");
}

#[test]
fn write_report_opens_the_file_private_to_the_owner() {
    let report_dir = tempfile::tempdir().unwrap();

    let path = write_report("secret", "s", report_dir.path()).unwrap();

    assert_eq!(mode_of(&path), 0o600);
}

#[test]
fn write_report_replaces_a_wider_older_report_and_makes_it_private() {
    let report_dir = tempfile::tempdir().unwrap();
    let old = report_dir.path().join("claude-s.md");
    fs::write(&old, "an older and much longer report").unwrap();
    fs::set_permissions(&old, fs::Permissions::from_mode(0o644)).unwrap();

    let path = write_report("new", "s", report_dir.path()).unwrap();

    assert_eq!(path, old);
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(mode_of(&path), 0o600);
}

#[rstest]
#[case::slash("a/b")]
#[case::backslash("a\\b")]
#[case::parent("../escape")]
#[case::absolute("/etc/passwd")]
#[case::trailing_slash("a/")]
fn write_report_refuses_a_session_id_that_is_not_a_file_name(#[case] session_id: &str) {
    let home = tempfile::tempdir().unwrap();
    let report_dir = home.path().join("debug");

    let error = write_report("x", session_id, &report_dir).unwrap_err();

    assert!(
        matches!(&error, DebugError::SessionIdNotFileName { session_id: found } if found == session_id)
    );
    assert!(!report_dir.exists());
}

#[rstest]
#[case::uuid("0f8fad5b-d9cb-469f-a165-70867728950e")]
#[case::dots("a.b")]
#[case::spaces("a b")]
#[case::unicode("sess-\u{e9}")]
fn write_report_keeps_every_other_session_id_in_the_file_name(#[case] session_id: &str) {
    let report_dir = tempfile::tempdir().unwrap();

    let path = write_report("x", session_id, report_dir.path()).unwrap();

    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        format!("claude-{session_id}.md")
    );
}

#[test]
fn write_report_reports_a_directory_it_cannot_create() {
    let home = tempfile::tempdir().unwrap();
    let blocker = home.path().join("file");
    fs::write(&blocker, "").unwrap();

    let error = write_report("x", "s", &blocker.join("debug")).unwrap_err();

    assert!(
        matches!(&error, DebugError::ReportDirectory { path, .. } if path.ends_with("file/debug"))
    );
    assert!(error.source().is_some());
}

#[test]
fn write_report_reports_a_report_path_it_cannot_open() {
    let report_dir = tempfile::tempdir().unwrap();
    fs::create_dir(report_dir.path().join("claude-s.md")).unwrap();

    let error = write_report("x", "s", report_dir.path()).unwrap_err();

    assert!(
        matches!(&error, DebugError::ReportCreate { path, .. } if path.ends_with("claude-s.md"))
    );
    assert!(error.source().is_some());
}
