#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::sse::{
    mark_sse_poller_spawned, run_id_from_sse_frames, run_id_from_sse_lines, split_lines,
    sse_poller_spawned,
};
use super::support::logging_object;

const FIRST_WINS: &[u8] = include_bytes!("../../../../fuzz/corpus/tinyfish_sse_frames/first-wins");
const INVALID_JSON: &[u8] =
    include_bytes!("../../../../fuzz/corpus/tinyfish_sse_frames/invalid-json");
const MALFORMED_UTF8: &[u8] =
    include_bytes!("../../../../fuzz/corpus/tinyfish_sse_frames/malformed-utf8");
const MISSING_RUN_ID: &[u8] =
    include_bytes!("../../../../fuzz/corpus/tinyfish_sse_frames/missing-run-id");
const VALID_RUN_ID: &[u8] =
    include_bytes!("../../../../fuzz/corpus/tinyfish_sse_frames/valid-run-id");

#[rstest]
#[case::the_first_run_id_wins(
    b"data: {\"run_id\": \"first\"}\ndata: {\"run_id\": \"second\"}\n",
    Some("first")
)]
#[case::a_status_frame_comes_first(
    b"data: {\"status\": \"COMPLETED\"}\ndata: {\"run_id\": \"second\"}\n",
    Some("second")
)]
#[case::non_data_lines_are_skipped(
    b"event: message\ndata: {\"run_id\": \"run-1\"}\n",
    Some("run-1")
)]
#[case::invalid_json_is_skipped(b"data: not json\ndata: {\"run_id\": \"run-1\"}\n", Some("run-1"))]
#[case::no_space_after_the_colon(b"data:{\"run_id\":\"tight\"}", Some("tight"))]
#[case::a_leading_space_skips_the_line(b" data: {\"run_id\": \"spaced\"}\n", None)]
#[case::an_empty_run_id_is_skipped(b"data: {\"run_id\": \"\"}\n", None)]
#[case::a_null_run_id_is_skipped(b"data: {\"run_id\": null}\n", None)]
#[case::a_number_run_id_is_skipped(b"data: {\"run_id\": 5}\n", None)]
#[case::a_run_without_a_run_id(b"data: {\"status\": \"PENDING\"}\n", None)]
#[case::a_json_list_frame(b"data: [1, 2]\ndata: {\"run_id\": \"after\"}\n", Some("after"))]
#[case::a_form_feed_splits_frames(
    b"data: {\"run_id\": \"a\"}\x0cdata: {\"run_id\": \"b\"}",
    Some("a")
)]
#[case::crlf_frames(b"data: {\"run_id\": \"crlf\"}\r\n", Some("crlf"))]
#[case::a_carriage_return_splits_frames(
    b"data: {\"status\": \"PENDING\"}\rdata: {\"run_id\": \"after_cr\"}",
    Some("after_cr")
)]
#[case::surrounding_spaces_are_trimmed(b"data:   {\"run_id\": \"trimmed\"}  \n", Some("trimmed"))]
#[case::a_unit_separator_pads_the_frame(b"data:\x1f{\"run_id\": \"unit\"}\x1f\n", Some("unit"))]
#[case::a_bare_data_line(b"data:\n", None)]
#[case::no_frames(b"", None)]
#[case::a_malformed_byte_becomes_a_replacement_character(
    b"data: {\"run_id\": \"\xff\"}\n",
    Some("\u{fffd}")
)]
fn extracts_the_first_run_id(#[case] frames: &[u8], #[case] expected: Option<&str>) {
    assert_eq!(
        run_id_from_sse_frames(frames),
        expected.map(ToOwned::to_owned)
    );
}

#[test]
fn reads_a_collected_stream_as_lines() {
    let lines = [
        String::from("event: message"),
        String::from("data: {\"run_id\": \"collected\"}"),
    ];
    assert_eq!(run_id_from_sse_lines(&lines), Some("collected".to_owned()));
}

#[rstest]
#[case::a_unix_break("a\nb", &["a", "b"])]
#[case::a_crlf_pair("a\r\nb", &["a", "b"])]
#[case::a_lone_carriage_return("a\rb", &["a", "b"])]
#[case::two_carriage_returns_in_a_row("a\r\rb", &["a", "", "b"])]
#[case::crlf_pairs_back_to_back("a\r\n\r\nb", &["a", "", "b"])]
#[case::a_blank_line_between("a\n\nb", &["a", "", "b"])]
#[case::a_line_separator("a\u{2028}b", &["a", "b"])]
#[case::no_trailing_empty_line("a\n", &["a"])]
#[case::no_text("", &[])]
fn splits_lines_like_python(#[case] text: &str, #[case] expected: &[&str]) {
    assert_eq!(split_lines(text), expected);
}

#[test]
fn replays_the_tinyfish_sse_frames_corpus() {
    assert_eq!(run_id_from_sse_frames(FIRST_WINS).as_deref(), Some("first"));
    assert_eq!(run_id_from_sse_frames(INVALID_JSON), None);
    assert_eq!(
        run_id_from_sse_frames(MALFORMED_UTF8).as_deref(),
        Some("\u{fffd}")
    );
    assert_eq!(run_id_from_sse_frames(MISSING_RUN_ID), None);
    assert_eq!(
        run_id_from_sse_frames(VALID_RUN_ID).as_deref(),
        Some("run-1")
    );
}

#[test]
fn marks_and_reads_a_spawned_sse_poller() {
    let logging_obj = logging_object();
    assert!(!sse_poller_spawned(&logging_obj));
    mark_sse_poller_spawned(&logging_obj);
    assert!(sse_poller_spawned(&logging_obj));
}

#[test]
fn reads_the_marker_litellm_writes_on_the_logging_details() {
    let logging_obj = logging_object();
    logging_obj.edit(&mut |state| {
        state
            .model_call_details
            .insert("tinyfish_sse_poller_spawned".to_owned(), Json::Bool(true));
    });
    assert!(sse_poller_spawned(&logging_obj));
}

#[test]
fn a_truthy_detail_other_than_true_is_not_a_spawned_poller() {
    let logging_obj = logging_object();
    logging_obj.edit(&mut |state| {
        state.model_call_details.insert(
            "tinyfish_sse_poller_spawned".to_owned(),
            Json::Text("yes".to_owned()),
        );
    });
    assert!(!sse_poller_spawned(&logging_obj));
}
