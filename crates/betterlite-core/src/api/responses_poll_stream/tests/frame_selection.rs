use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{self, Answer, AnswerStep, Scenario};
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_poll_stream::frame::{
    PollFrame, PollFrameError, decode_poll_stream_frame,
};

const ITEM_ADDED: &str =
    r#"{"item":{"content":[],"id":"m1","type":"message"},"type":"response.output_item.added"}"#;
const PART_ADDED: &str = r#"{"item_id":"m1","part":{"text":"","type":"output_text"},"type":"response.content_part.added"}"#;
const DELTA: &str =
    r#"{"content_index":0,"delta":"A","item_id":"m1","type":"response.output_text.delta"}"#;

#[rstest]
#[case::json_event("data: {\"type\":\"x\"}\n\n", "event")]
#[case::done("data: [DONE]\n\n", "done")]
#[case::trimmed_done("data:  [DONE] \r\n", "done")]
#[case::no_space("data:{\"type\":\"x\"}", "no_data")]
#[case::comment(": keep-alive\n\n", "no_data")]
#[case::empty("", "no_data")]
#[case::newline_only("\n\n", "no_data")]
#[case::first_of_two("data: {\"type\":\"first\"}\ndata: {\"type\":\"second\"}", "event")]
#[case::event_line_first("event: message\ndata: {\"type\":\"x\"}", "event")]
#[case::malformed("data: {bad}\n\n", "malformed")]
#[case::scalar("data: 42", "event")]
#[case::uppercase_prefix("Data: {}", "no_data")]
#[case::unicode_break("data: 1\u{2028}data: 2", "event")]
#[case::carriage_return("data: {\"type\":\"x\"}\rdata: {\"type\":\"y\"}", "event")]
#[case::single_line("data: {\"type\":\"x\"}", "event")]
#[case::python_whitespace("data: \u{a0}{\"type\":\"x\"}\u{1c}", "event")]
fn selects_literal_frames(#[case] frame: &str, #[case] expected: &str) {
    let kind = match decode_poll_stream_frame(frame.as_bytes(), ApiLimits::default()) {
        Ok(PollFrame::NoData) => "no_data",
        Ok(PollFrame::Done) => "done",
        Ok(PollFrame::Event(_)) => "event",
        Ok(PollFrame::Malformed) => "malformed",
        Err(_) => "error",
    };
    assert_eq!(kind, expected);
}

#[test]
fn keeps_the_first_data_line() {
    let frame = b"data: {\"type\":\"first\"}\ndata: {\"type\":\"second\"}";
    let decoded = decode_poll_stream_frame(frame, ApiLimits::default()).expect("the frame decodes");
    let PollFrame::Event(event) = decoded else {
        panic!("the frame carries an event");
    };
    assert_eq!(support::text(&event, "type"), "first");
}

#[test]
fn rejects_non_utf8() {
    let error = decode_poll_stream_frame(b"data: \xff", ApiLimits::default())
        .expect_err("the frame is not UTF-8");
    assert_eq!(error, PollFrameError::Utf8);
}

#[test]
fn bounds_frame_bytes() {
    let oversized = vec![b'x'; ApiLimits::default().max_event_bytes.saturating_add(1)];
    let error = decode_poll_stream_frame(&oversized, ApiLimits::default())
        .expect_err("the frame is over the event limit");
    assert!(matches!(error, PollFrameError::Limit(_)));
}

pub(super) async fn run_chunks(chunks: Vec<Vec<u8>>, readings: &[u64]) -> (Scenario, Vec<String>) {
    let steps: Vec<AnswerStep> = chunks.into_iter().map(AnswerStep::Chunk).collect();
    let (guard, logs) = support::capture_logs();
    let mut scenario = support::scenario(readings, Answer::Streaming(steps));
    scenario
        .run()
        .await
        .expect("the worker records its outcome");
    drop(guard);
    let warnings = logs.messages("WARN");
    (scenario, warnings)
}

pub(super) async fn split_payload() -> (Scenario, Vec<String>) {
    let mut chunks = vec![
        b"data: {\"type\":\"response.comp".to_vec(),
        b"leted\"}\n\n".to_vec(),
    ];
    chunks.extend(support::frames(&[
        r#"{"response":{"status":"completed"},"type":"response.completed"}"#,
    ]));
    run_chunks(chunks, &[1000]).await
}

pub(super) async fn scalar_payload() -> (Scenario, Vec<String>) {
    run_chunks(support::frames(&["42"]), &[1000]).await
}

pub(super) async fn invalid_utf8() -> (Scenario, Vec<String>) {
    run_chunks(vec![b"data: \xff\xfe".to_vec()], &[1000]).await
}

pub(super) async fn done_before_terminal() -> (Scenario, Vec<String>) {
    let mut chunks = support::frames(&[ITEM_ADDED, "[DONE]"]);
    chunks.extend(support::frames(&[
        r#"{"response":{"error":{"code":"x"},"status":"failed"},"type":"response.failed"}"#,
    ]));
    run_chunks(chunks, &[1000]).await
}

pub(super) async fn unknown_event_flushes() -> (Scenario, Vec<String>) {
    let chunks = support::frames(&[
        ITEM_ADDED,
        PART_ADDED,
        r#"{"type":"response.something_else"}"#,
        DELTA,
        r#"{"response":{"status":"completed"},"type":"response.completed"}"#,
    ]);
    run_chunks(chunks, &[1000, 1000, 1000, 1200, 1300, 1400, 1500]).await
}

pub(super) async fn non_text_status() -> Scenario {
    let status = r#"{"response":{"status":5},"type":"response.completed"}"#;
    let (scenario, _warnings) = run_chunks(support::frames(&[status]), &[1000]).await;
    scenario
}

#[tokio::test]
async fn non_text_terminal_status_fails_the_record() {
    let scenario = non_text_status().await;
    assert_failed_message(
        &scenario,
        "the streaming event field status is not of the declared type",
    );
}

pub(super) fn assert_warned(warnings: &[String], needle: &str) {
    assert!(
        warnings.iter().any(|warning| warning.contains(needle)),
        "no warning contains {needle}: {warnings:?}"
    );
}

pub(super) fn assert_failed_message(scenario: &Scenario, message: &str) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "failed");
    let error = support::field(&record, "error");
    assert_eq!(support::text(&error, "type"), "internal_error");
    assert_eq!(support::text(&error, "code"), "background_streaming_error");
    assert_eq!(support::text(&error, "message"), message);
}

pub(super) fn assert_completed(scenario: &Scenario) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "completed");
    assert_eq!(support::field(&record, "error"), Json::Null);
}

pub(super) fn assert_flushed_before_final(scenario: &Scenario) {
    let writes = scenario.cache.writes();
    assert_eq!(writes.len(), 4);
    assert_eq!(support::text(&writes[0], "status"), "in_progress");
    assert_eq!(support::text(&writes[1], "status"), "in_progress");
    assert_eq!(support::first_text(&writes[1]), "");
    assert_eq!(support::first_text(&writes[2]), "A");
    assert_eq!(support::text(&writes[3], "status"), "completed");
    assert_eq!(support::first_text(&writes[3]), "A");
}
