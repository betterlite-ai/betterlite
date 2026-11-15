#![cfg(test)]

use std::fs;
use std::iter::repeat_n;
use std::path::{Path, PathBuf};

use rstest::rstest;
use tempfile::TempDir;

use super::{SCAN_LIMIT_BYTES, latest_transcript_model, model_label};

fn assistant(model: &str) -> String {
    format!(r#"{{"type":"assistant","message":{{"model":"{model}"}}}}"#)
}

fn transcript(directory: &TempDir, content: &[u8]) -> PathBuf {
    let path = directory.path().join("transcript.jsonl");
    fs::write(&path, content).unwrap();
    path
}

fn latest(lines: &[&str]) -> Option<String> {
    let directory = TempDir::new().unwrap();
    let path = transcript(&directory, lines.join("\n").as_bytes());
    latest_transcript_model(&path)
}

#[rstest]
#[case::newest_assistant_line_wins(
    &[r#"{"type":"assistant","message":{"model":"old"}}"#, r#"{"type":"assistant","message":{"model":"new"}}"#],
    Some("new")
)]
#[case::user_lines_are_skipped(
    &[r#"{"type":"assistant","message":{"model":"served"}}"#, r#"{"type":"user","message":{"model":"typed"}}"#],
    Some("served")
)]
#[case::missing_type_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"served"}}"#, r#"{"message":{"model":"typed"}}"#],
    Some("served")
)]
#[case::sidechain_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"main"}}"#, r#"{"type":"assistant","isSidechain":true,"message":{"model":"side"}}"#],
    Some("main")
)]
#[case::explicit_false_sidechain_counts(
    &[r#"{"type":"assistant","message":{"model":"main"}}"#, r#"{"type":"assistant","isSidechain":false,"message":{"model":"later"}}"#],
    Some("later")
)]
#[case::agent_id_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"main"}}"#, r#"{"type":"assistant","agentId":"a1","message":{"model":"agent"}}"#],
    Some("main")
)]
#[case::empty_agent_id_counts(
    &[r#"{"type":"assistant","message":{"model":"main"}}"#, r#"{"type":"assistant","agentId":"","message":{"model":"later"}}"#],
    Some("later")
)]
#[case::null_agent_id_counts(
    &[r#"{"type":"assistant","message":{"model":"main"}}"#, r#"{"type":"assistant","agentId":null,"message":{"model":"later"}}"#],
    Some("later")
)]
#[case::synthetic_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"real"}}"#, r#"{"type":"assistant","message":{"model":"<synthetic>"}}"#],
    Some("real")
)]
#[case::synthetic_hidden_behind_control_characters_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"real"}}"#, r#"{"type":"assistant","message":{"model":"<synth\u0007etic>"}}"#],
    Some("real")
)]
#[case::synthetic_prefix_is_a_model(
    &[r#"{"type":"assistant","message":{"model":"<synthetic>-2"}}"#],
    Some("<synthetic>-2")
)]
#[case::empty_model_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"real"}}"#, r#"{"type":"assistant","message":{"model":""}}"#],
    Some("real")
)]
#[case::model_that_prints_as_nothing_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"real"}}"#, r#"{"type":"assistant","message":{"model":"\u001b\u0007"}}"#],
    Some("real")
)]
#[case::missing_message_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"real"}}"#, r#"{"type":"assistant"}"#],
    Some("real")
)]
#[case::message_without_model_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"real"}}"#, r#"{"type":"assistant","message":{"role":"assistant"}}"#],
    Some("real")
)]
#[case::numeric_model_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"real"}}"#, r#"{"type":"assistant","message":{"model":7}}"#],
    Some("real")
)]
#[case::malformed_json_is_skipped(
    &[r#"{"type":"assistant","message":{"model":"real"}}"#, r#"{"type":"assistant","message":{"model""#, "not json", ""],
    Some("real")
)]
#[case::control_characters_are_stripped(
    &[r#"{"type":"assistant","message":{"model":"a\u001b[31mb\u0007"}}"#],
    Some("a[31mb")
)]
#[case::nothing_qualifies(
    &[r#"{"type":"user","message":{"model":"typed"}}"#, r#"{"type":"assistant","isSidechain":true,"message":{"model":"side"}}"#],
    None
)]
#[case::empty_file(&[], None)]
fn the_latest_foreground_assistant_model(#[case] lines: &[&str], #[case] expected: Option<&str>) {
    assert_eq!(latest(lines).as_deref(), expected);
}

#[test]
fn a_trailing_newline_changes_nothing() {
    let directory = TempDir::new().unwrap();
    let content = format!("{}\n{}\n", assistant("old"), assistant("new"));

    let path = transcript(&directory, content.as_bytes());

    assert_eq!(latest_transcript_model(&path).as_deref(), Some("new"));
}

#[test]
fn a_missing_transcript_has_no_model() {
    let directory = TempDir::new().unwrap();

    assert_eq!(
        latest_transcript_model(&directory.path().join("absent")),
        None
    );
}

#[test]
fn a_directory_is_not_a_transcript() {
    let directory = TempDir::new().unwrap();

    assert_eq!(latest_transcript_model(directory.path()), None);
}

#[rstest]
#[case::line_ends_exactly_at_the_window_start(0, Some("edge"))]
#[case::first_byte_of_the_line_is_outside_the_window(1, None)]
#[case::line_is_far_outside_the_window(SCAN_LIMIT_BYTES, None)]
fn only_the_last_four_mebibytes_are_scanned(#[case] overflow: u64, #[case] expected: Option<&str>) {
    let directory = TempDir::new().unwrap();
    let line = format!("{}\n", assistant("edge"));
    let line_bytes = u64::try_from(line.len()).unwrap();
    let filler = SCAN_LIMIT_BYTES
        .saturating_sub(line_bytes)
        .saturating_add(overflow);
    let mut content = line.into_bytes();
    content.extend(repeat_n(b'x', usize::try_from(filler).unwrap()));

    let path = transcript(&directory, &content);

    assert_eq!(latest_transcript_model(&path).as_deref(), expected);
}

#[test]
fn a_line_inside_the_window_is_found_past_a_larger_file_head() {
    let directory = TempDir::new().unwrap();
    let mut content = vec![b'x'; usize::try_from(SCAN_LIMIT_BYTES).unwrap()];
    content.push(b'\n');
    content.extend_from_slice(assistant("tail").as_bytes());

    let path = transcript(&directory, &content);

    assert_eq!(latest_transcript_model(&path).as_deref(), Some("tail"));
}

fn labelled(models: &str, model: &str) -> String {
    let directory = TempDir::new().unwrap();
    let cache = directory.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("gateway-models.json"), models).unwrap();
    model_label(model, directory.path())
}

#[rstest]
#[case::full_id_matches(
    r#"{"models":[{"id":"anthropic/claude-x","display_name":"Claude X"}]}"#,
    "anthropic/claude-x",
    "Claude X"
)]
#[case::bare_id_matches_a_prefixed_model(
    r#"{"models":[{"id":"claude-x","display_name":"Claude X"}]}"#,
    "anthropic/claude-x",
    "Claude X"
)]
#[case::plain_id_matches(
    r#"{"models":[{"id":"claude-x","display_name":"Claude X"}]}"#,
    "claude-x",
    "Claude X"
)]
#[case::no_match_falls_back_to_the_bare_id(
    r#"{"models":[{"id":"other","display_name":"Other"}]}"#,
    "anthropic/claude-x",
    "claude-x"
)]
#[case::bare_id_is_the_last_path_segment(r#"{"models":[]}"#, "a/b/claude-x", "claude-x")]
#[case::first_match_wins(
    r#"{"models":[{"id":"claude-x","display_name":"First"},{"id":"claude-x","display_name":"Second"}]}"#,
    "claude-x",
    "First"
)]
#[case::empty_display_name_is_skipped(
    r#"{"models":[{"id":"claude-x","display_name":""},{"id":"claude-x","display_name":"Second"}]}"#,
    "claude-x",
    "Second"
)]
#[case::display_name_that_prints_as_nothing_is_skipped(
    r#"{"models":[{"id":"claude-x","display_name":"\u001b"},{"id":"claude-x","display_name":"Second"}]}"#,
    "claude-x",
    "Second"
)]
#[case::display_name_is_stripped_of_control_characters(
    r#"{"models":[{"id":"claude-x","display_name":"Cl\u001b[1maude"}]}"#,
    "claude-x",
    "Cl[1maude"
)]
#[case::malformed_entries_do_not_hide_later_ones(
    r#"{"models":["text",7,null,{"id":7,"display_name":"Seven"},{"id":"claude-x"},{"id":"claude-x","display_name":null},{"id":"claude-x","display_name":"Claude X"}]}"#,
    "claude-x",
    "Claude X"
)]
#[case::models_that_is_not_a_list(r#"{"models":{"id":"claude-x"}}"#, "claude-x", "claude-x")]
#[case::missing_models(r#"{"other":[]}"#, "claude-x", "claude-x")]
#[case::not_an_object("[1,2]", "claude-x", "claude-x")]
#[case::not_json("{nope", "claude-x", "claude-x")]
fn a_model_is_labelled_from_the_gateway_model_cache(
    #[case] models: &str,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(labelled(models, model), expected);
}

#[test]
fn a_missing_model_cache_falls_back_to_the_bare_id() {
    let directory = TempDir::new().unwrap();

    assert_eq!(
        model_label("anthropic/claude-x", directory.path()),
        "claude-x"
    );
    assert_eq!(
        model_label("claude-x", Path::new("/nonexistent/config")),
        "claude-x"
    );
}

#[test]
fn an_unreadable_model_cache_falls_back_to_the_bare_id() {
    let directory = TempDir::new().unwrap();
    fs::create_dir_all(directory.path().join("cache").join("gateway-models.json")).unwrap();

    assert_eq!(
        model_label("anthropic/claude-x", directory.path()),
        "claude-x"
    );
}
