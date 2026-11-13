#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{RunEvent, RunState};
use crate::watsonx_run::accumulator::{LineAccumulator, accumulate_text};
use crate::watsonx_transform::extract::extract_text_from_wxo_result;

fn decode(body: &str) -> RunEvent {
    serde_json::from_str(body).unwrap()
}

#[rstest]
#[case::completed("completed", Some(RunState::Completed))]
#[case::succeeded("succeeded", Some(RunState::Succeeded))]
#[case::failed("failed", Some(RunState::Failed))]
#[case::error("error", Some(RunState::Error))]
#[case::cancelled("cancelled", Some(RunState::Cancelled))]
#[case::running("running", None)]
#[case::pending("pending", None)]
#[case::empty("", None)]
#[case::capitalised("Completed", None)]
fn terminal_states_are_a_closed_set(#[case] status: &str, #[case] expected: Option<RunState>) {
    assert_eq!(RunState::parse(status), expected);
}

#[test]
fn decodes_the_run_id_and_status_of_a_run_body() {
    let event = decode(r#"{"run_id": "run-1", "status": "completed"}"#);
    assert_eq!(event.run_id.as_deref(), Some("run-1"));
    assert_eq!(event.status.as_deref(), Some("completed"));
    assert_eq!(event.id, None);
}

#[test]
fn keeps_the_text_locations_of_a_run_event() {
    let event =
        decode(r#"{"status": "completed", "last_message": {"content": [{"text": "hello"}]}}"#);
    assert_eq!(extract_text_from_wxo_result(&event.run), "hello");
}

#[rstest]
#[case::truncated(r#"{"status": "#)]
#[case::status_is_a_number(r#"{"status": 5}"#)]
#[case::run_id_is_a_list(r#"{"run_id": [1]}"#)]
#[case::id_is_a_bool(r#"{"id": true}"#)]
#[case::array("[1, 2]")]
#[case::text("plain text")]
#[case::empty("")]
fn malformed_run_events_are_errors_not_panics(#[case] body: &str) {
    serde_json::from_str::<RunEvent>(body).unwrap_err();
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"status": null, "run_id": null, "id": null}"#)]
fn absent_or_null_run_fields_take_defaults(#[case] body: &str) {
    let event = decode(body);
    assert_eq!(event.status.as_deref(), None);
    assert_eq!(event.run_id, None);
    assert_eq!(event.id, None);
}

#[test]
fn unknown_run_fields_are_ignored() {
    let event = decode(r#"{"status": "running", "extra": {"deep": [1]}}"#);
    assert_eq!(event.status.as_deref(), Some("running"));
    assert_eq!(event.run_id, None);
    assert_eq!(event.id, None);
}

#[rstest]
fn replays_the_watsonx_run_event_corpus(#[files("fuzz/corpus/watsonx_run_event/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(serde_json::from_slice::<RunEvent>(&bytes));
    let mut accumulator = LineAccumulator::default();
    let mut lines = Vec::new();
    for chunk in bytes.chunks(3) {
        if let Ok(mut complete) = accumulator.push(chunk) {
            lines.append(&mut complete);
        }
    }
    if let Some(line) = accumulator.finish() {
        lines.push(line);
    }
    drop(accumulate_text(&lines));
}
