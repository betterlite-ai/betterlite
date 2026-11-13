#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use rstest::rstest;

use super::detect_claude_session_id;
use crate::commands::debug::error::DebugError;

const OLDER: &str = "11111111-aaaa-bbbb-cccc-111111111111";
const NEWER: &str = "22222222-aaaa-bbbb-cccc-222222222222";

fn environment(value: Option<&str>) -> BTreeMap<String, String> {
    value
        .map(|id| ("CLAUDE_CODE_SESSION_ID".to_owned(), id.to_owned()))
        .into_iter()
        .collect()
}

fn write_transcript(claude_dir: &Path, project: &str, file_name: &str, age_seconds: u64) {
    let directory = claude_dir.join("projects").join(project);
    fs::create_dir_all(&directory).unwrap();
    let file = fs::File::create(directory.join(file_name)).unwrap();
    let modified = SystemTime::now()
        .checked_sub(Duration::from_secs(age_seconds))
        .unwrap();
    file.set_modified(modified).unwrap();
}

#[rstest]
#[case::plain("abc-123")]
#[case::uuid(NEWER)]
fn session_id_prefers_the_environment_variable(#[case] id: &str) {
    let claude_dir = tempfile::tempdir().unwrap();
    write_transcript(claude_dir.path(), "p", &format!("{OLDER}.jsonl"), 5);

    let found = detect_claude_session_id(&environment(Some(id)), claude_dir.path()).unwrap();

    assert_eq!(found.as_deref(), Some(id));
}

#[test]
fn session_id_ignores_an_empty_environment_variable() {
    let claude_dir = tempfile::tempdir().unwrap();
    write_transcript(claude_dir.path(), "p", &format!("{OLDER}.jsonl"), 5);

    let found = detect_claude_session_id(&environment(Some("")), claude_dir.path()).unwrap();

    assert_eq!(found.as_deref(), Some(OLDER));
}

#[test]
fn session_id_takes_the_newest_transcript_across_projects() {
    let claude_dir = tempfile::tempdir().unwrap();
    write_transcript(claude_dir.path(), "first", &format!("{NEWER}.jsonl"), 10);
    write_transcript(claude_dir.path(), "second", &format!("{OLDER}.jsonl"), 500);
    write_transcript(
        claude_dir.path(),
        "third",
        "33333333-aaaa-bbbb-cccc-333333333333.jsonl",
        900,
    );

    let found = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap();

    assert_eq!(found.as_deref(), Some(NEWER));
}

#[test]
fn session_id_takes_the_newest_transcript_inside_one_project() {
    let claude_dir = tempfile::tempdir().unwrap();
    write_transcript(claude_dir.path(), "p", &format!("{OLDER}.jsonl"), 10);
    write_transcript(claude_dir.path(), "p", &format!("{NEWER}.jsonl"), 500);

    let found = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap();

    assert_eq!(found.as_deref(), Some(OLDER));
}

#[rstest]
#[case::not_jsonl("11111111-aaaa-bbbb-cccc-111111111111.json")]
#[case::wrong_extension("11111111-aaaa-bbbb-cccc-111111111111.txt")]
#[case::no_extension("11111111-aaaa-bbbb-cccc-111111111111")]
#[case::uppercase("11111111-AAAA-BBBB-CCCC-111111111111.jsonl")]
#[case::not_hex("11111111-gggg-bbbb-cccc-111111111111.jsonl")]
#[case::short_first_group("1111111-aaaa-bbbb-cccc-111111111111.jsonl")]
#[case::long_last_group("11111111-aaaa-bbbb-cccc-1111111111111.jsonl")]
#[case::short_last_group("11111111-aaaa-bbbb-cccc-11111111111.jsonl")]
#[case::four_groups("11111111-aaaa-bbbb-111111111111.jsonl")]
#[case::six_groups("11111111-aaaa-bbbb-cccc-1111-11111111.jsonl")]
#[case::no_dashes("11111111aaaabbbbcccc111111111111.jsonl")]
#[case::agent_transcript("agent-11111111.jsonl")]
fn session_id_skips_files_that_are_not_session_transcripts(#[case] file_name: &str) {
    let claude_dir = tempfile::tempdir().unwrap();
    write_transcript(claude_dir.path(), "p", file_name, 5);

    let found = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap();

    assert_eq!(found, None);
}

#[test]
fn session_id_keeps_a_valid_transcript_next_to_skipped_ones() {
    let claude_dir = tempfile::tempdir().unwrap();
    write_transcript(claude_dir.path(), "p", "notes.jsonl", 1);
    write_transcript(claude_dir.path(), "p", &format!("{OLDER}.jsonl"), 500);

    let found = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap();

    assert_eq!(found.as_deref(), Some(OLDER));
}

#[test]
fn session_id_accepts_all_hex_digits() {
    let claude_dir = tempfile::tempdir().unwrap();
    write_transcript(
        claude_dir.path(),
        "p",
        "01234567-89ab-cdef-0123-456789abcdef.jsonl",
        5,
    );

    let found = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap();

    assert_eq!(
        found.as_deref(),
        Some("01234567-89ab-cdef-0123-456789abcdef")
    );
}

#[test]
fn session_id_ignores_transcripts_directly_under_projects() {
    let claude_dir = tempfile::tempdir().unwrap();
    let projects = claude_dir.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    fs::write(projects.join(format!("{OLDER}.jsonl")), "").unwrap();

    let found = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap();

    assert_eq!(found, None);
}

#[test]
fn session_id_ignores_transcripts_nested_deeper_than_a_project() {
    let claude_dir = tempfile::tempdir().unwrap();
    write_transcript(claude_dir.path(), "p/nested", &format!("{OLDER}.jsonl"), 5);

    let found = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap();

    assert_eq!(found, None);
}

#[test]
fn session_id_is_none_without_a_projects_directory() {
    let claude_dir = tempfile::tempdir().unwrap();

    let found = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap();

    assert_eq!(found, None);
}

#[test]
fn session_id_is_none_without_a_claude_directory() {
    let claude_dir = tempfile::tempdir().unwrap();

    let found =
        detect_claude_session_id(&environment(None), &claude_dir.path().join("absent")).unwrap();

    assert_eq!(found, None);
}

#[test]
fn session_id_reports_a_projects_path_that_is_not_a_directory() {
    let claude_dir = tempfile::tempdir().unwrap();
    fs::write(claude_dir.path().join("projects"), "").unwrap();

    let error = detect_claude_session_id(&environment(None), claude_dir.path()).unwrap_err();

    assert!(
        matches!(&error, DebugError::TranscriptScan { path, .. } if path.ends_with("projects"))
    );
    assert!(error.source().is_some());
}
