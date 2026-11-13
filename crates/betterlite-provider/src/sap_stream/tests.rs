#![cfg(test)]

use std::fs;

use betterlite_openai::stream::chunk::Chunk;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{Mode, SapLineStream, SapStep, SapStreamError, add_stream_flag, event_to_chunk};

const FIXED_NOW: i64 = 1_700_000_000;

#[derive(Debug)]
struct ReadFailure;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn chunk_json(chunk: &Chunk) -> Json {
    serde_json::from_value(serde_json::to_value(chunk).unwrap()).unwrap()
}

fn mapped(event: &str, now: i64) -> Option<Json> {
    event_to_chunk(&json(event), now)
        .unwrap()
        .map(|chunk| chunk_json(&chunk))
}

fn rejected(event: &str, now: i64) -> SapStreamError {
    event_to_chunk(&json(event), now).unwrap_err()
}

fn new_stream() -> SapLineStream {
    SapLineStream {
        now: FIXED_NOW,
        done: false,
    }
}

fn next(stream: &mut SapLineStream, line: &str, mode: Mode) -> Result<SapStep, SapStreamError> {
    stream.next_chunk(line, mode)
}

#[test]
fn maps_an_orchestration_result_with_defaults() {
    assert_eq!(
        mapped(
            r#"{"orchestration_result":{"choices":[{"delta":{"content":"a"}}]}}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"stream-chunk","object":"chat.completion.chunk","created":1700000000,"model":"unknown","choices":[{"index":0,"delta":{"content":"a"}}]}"#
        ))
    );
}

#[test]
fn keeps_orchestration_result_values_and_choice_fields() {
    assert_eq!(
        mapped(
            r#"{"orchestration_result":{"id":"r1","created":11,"model":"m1","choices":[{"index":1,"delta":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"r1","object":"chat.completion.chunk","created":11,"model":"m1","choices":[{"index":1,"delta":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#
        ))
    );
}

#[test]
fn drops_choice_fields_beyond_index_delta_and_finish_reason() {
    assert_eq!(
        mapped(
            r#"{"orchestration_result":{"choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null,"text":"dropped","logprobs":{"content":[]}}]}}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"stream-chunk","object":"chat.completion.chunk","created":1700000000,"model":"unknown","choices":[{"index":0,"delta":{"content":"a"}}]}"#
        ))
    );
}

#[rstest]
#[case::final_result(
    r#"{"final_result":{"id":"c","object":"other","created":7,"model":"m","choices":[]}}"#
)]
#[case::orchestration_result(
    r#"{"orchestration_result":{"id":"c","object":"other","created":7,"model":"m","choices":[]}}"#
)]
#[case::passthrough(r#"{"object":"other","id":"c","created":7,"model":"m","choices":[]}"#)]
fn forces_the_chunk_object(#[case] event: &str) {
    assert_eq!(
        mapped(event, FIXED_NOW),
        Some(json(
            r#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"m","choices":[]}"#
        ))
    );
}

#[test]
fn falls_back_to_request_id_then_created() {
    assert_eq!(
        mapped(
            r#"{"request_id":"req-1","created":11,"orchestration_result":{"choices":[]}}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"req-1","object":"chat.completion.chunk","created":11,"model":"unknown","choices":[]}"#
        ))
    );
}

#[test]
fn prefers_the_orchestration_result_identity_over_the_event() {
    let chunk = event_to_chunk(
        &json(
            r#"{"request_id":"req-1","created":11,"orchestration_result":{"id":"r1","created":12,"choices":[]}}"#,
        ),
        FIXED_NOW,
    )
    .unwrap()
    .unwrap();
    assert_eq!(chunk.id, "r1");
    assert_eq!(chunk.created, 12);
}

#[test]
fn drops_usage_and_fingerprint_from_an_orchestration_result() {
    assert_eq!(
        mapped(
            r#"{"orchestration_result":{"id":"r4","created":13,"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5},"system_fingerprint":"fp-1"}}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"r4","object":"chat.completion.chunk","created":13,"model":"unknown","choices":[]}"#
        ))
    );
}

#[rstest]
#[case::null(r#"{"orchestration_result":null}"#)]
#[case::empty_object(r#"{"orchestration_result":{}}"#)]
#[case::empty_text(r#"{"orchestration_result":""}"#)]
#[case::empty_list(r#"{"orchestration_result":[]}"#)]
#[case::heartbeat(r#"{"status":"ok"}"#)]
#[case::metrics(r#"{"object":"metrics","values":[1,2]}"#)]
fn gives_no_chunk_for_an_empty_or_unknown_event(#[case] event: &str) {
    assert_eq!(mapped(event, FIXED_NOW), None);
}

#[rstest]
#[case::list(r#"{"orchestration_result":[1]}"#)]
#[case::text(r#"{"orchestration_result":"x"}"#)]
#[case::number(r#"{"orchestration_result":0}"#)]
#[case::boolean(r#"{"orchestration_result":false}"#)]
fn rejects_an_orchestration_result_that_is_not_an_object(#[case] event: &str) {
    assert!(matches!(
        rejected(event, FIXED_NOW),
        SapStreamError::EventField("orchestration_result")
    ));
}

#[test]
fn parses_final_result_as_an_ordinary_chunk() {
    assert_eq!(
        mapped(
            r#"{"final_result":{"id":"c1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"x"},"finish_reason":"stop"}]}}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"c1","object":"chat.completion.chunk","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"x"},"finish_reason":"stop"}]}"#
        ))
    );
}

#[rstest]
#[case::null(r#"{"final_result":null}"#)]
#[case::empty_object(r#"{"final_result":{}}"#)]
#[case::missing_choices(r#"{"final_result":{"id":"c","created":7,"model":"m"}}"#)]
#[case::missing_model(r#"{"final_result":{"id":"c","created":7,"choices":[]}}"#)]
#[case::null_choices(r#"{"final_result":{"id":"c","created":7,"model":"m","choices":null}}"#)]
#[case::text(r#"{"final_result":"text"}"#)]
fn rejects_a_final_result_that_cannot_be_a_chunk(#[case] event: &str) {
    assert!(matches!(
        rejected(event, FIXED_NOW),
        SapStreamError::Chunk { .. } | SapStreamError::EventField(_)
    ));
}

#[test]
fn parses_a_passthrough_chunk_with_usage_and_fingerprint() {
    assert_eq!(
        mapped(
            r#"{"object":"chat.completion.chunk","id":"c2","created":8,"model":"m2","system_fingerprint":"fp-1","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"c2","object":"chat.completion.chunk","created":8,"model":"m2","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5},"system_fingerprint":"fp-1"}"#
        ))
    );
}

#[rstest]
#[case::passthrough(
    r#"{"object":"chat.completion.chunk","id":"c","created":1,"model":"m","choices":[],"extra":1}"#
)]
#[case::final_result(
    r#"{"final_result":{"id":"c","created":1,"model":"m","choices":[],"extra":1}}"#
)]
fn rejects_a_chunk_event_with_an_unknown_field(#[case] event: &str) {
    assert!(matches!(
        rejected(event, FIXED_NOW),
        SapStreamError::Chunk { .. }
    ));
}

#[rstest]
#[case::final_result_before_orchestration_result(
    r#"{"final_result":{"id":"f","created":1,"model":"m","choices":[]},"orchestration_result":{"id":"o","created":2,"model":"m2","choices":[]}}"#,
    r#"{"id":"f","object":"chat.completion.chunk","created":1,"model":"m","choices":[]}"#
)]
#[case::orchestration_result_before_passthrough(
    r#"{"object":"chat.completion.chunk","id":"p","created":3,"model":"m3","choices":[],"orchestration_result":{"id":"o","created":4,"model":"m4","choices":[]}}"#,
    r#"{"id":"o","object":"chat.completion.chunk","created":4,"model":"m4","choices":[]}"#
)]
fn prefers_the_earlier_event_shape(#[case] event: &str, #[case] expected: &str) {
    assert_eq!(mapped(event, FIXED_NOW), Some(json(expected)));
}

#[test]
fn accepts_a_finish_reason_outside_the_openai_literal_set() {
    assert_eq!(
        mapped(
            r#"{"orchestration_result":{"id":"x","choices":[{"delta":{},"finish_reason":"end_turn"}]}}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"x","object":"chat.completion.chunk","created":1700000000,"model":"unknown","choices":[{"index":0,"delta":{},"finish_reason":"end_turn"}]}"#
        ))
    );
}

#[test]
fn defaults_a_tool_call_arguments_to_empty() {
    assert_eq!(
        mapped(
            r#"{"object":"chat.completion.chunk","id":"c","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"t","type":"function","function":{"name":"f"}}]}}]}"#,
            FIXED_NOW,
        ),
        Some(json(
            r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"t","type":"function","function":{"name":"f","arguments":""}}]}}]}"#
        ))
    );
}

#[rstest]
#[case::error_null(r#"{"error":null}"#)]
#[case::code_zero(r#"{"code":0}"#)]
#[case::before_final_result(
    r#"{"error":"boom","final_result":{"id":"c","created":7,"model":"m","choices":[]}}"#
)]
#[case::before_orchestration_result(r#"{"code":1,"orchestration_result":{"choices":[]}}"#)]
fn rejects_an_event_with_a_code_or_error_key(#[case] event: &str) {
    assert!(matches!(
        rejected(event, FIXED_NOW),
        SapStreamError::Event { .. }
    ));
}

#[rstest]
#[case::array("[1,2,3]")]
#[case::text(r#""hello""#)]
#[case::empty_object("{}")]
#[case::choices_without_object(r#"{"choices":[]}"#)]
#[case::object_without_choices(r#"{"object":"chat.completion.chunk"}"#)]
fn ignores_values_that_are_not_chunk_events(#[case] event: &str) {
    assert_eq!(mapped(event, FIXED_NOW), None);
}

#[rstest]
#[case::null(r#"{"orchestration_result":{"choices":[{"index":null,"delta":{}}]}}"#)]
#[case::text(r#"{"orchestration_result":{"choices":[{"index":"1","delta":{}}]}}"#)]
#[case::negative(r#"{"orchestration_result":{"choices":[{"index":-1,"delta":{}}]}}"#)]
#[case::beyond_u32(r#"{"orchestration_result":{"choices":[{"index":4294967296,"delta":{}}]}}"#)]
fn rejects_a_choice_with_an_invalid_index(#[case] event: &str) {
    assert!(matches!(
        rejected(event, FIXED_NOW),
        SapStreamError::EventField(_)
    ));
}

#[rstest]
#[case::text(r#"{"orchestration_result":{"choices":[{"delta":"x"}]}}"#)]
#[case::list(r#"{"orchestration_result":{"choices":[{"delta":[1]}]}}"#)]
#[case::choices_text(r#"{"orchestration_result":{"choices":"x"}}"#)]
#[case::missing_choice(r#"{"orchestration_result":{"choices":["x"]}}"#)]
fn rejects_choices_or_deltas_that_are_not_objects(#[case] event: &str) {
    assert!(matches!(
        rejected(event, FIXED_NOW),
        SapStreamError::EventField(_)
    ));
}

#[rstest]
#[case::missing_delta(r#"{"orchestration_result":{"choices":[{"index":0}]}}"#)]
#[case::null_delta(r#"{"orchestration_result":{"choices":[{"index":0,"delta":null}]}}"#)]
#[case::empty_delta(r#"{"orchestration_result":{"choices":[{"index":0,"delta":{}}]}}"#)]
fn treats_a_missing_or_null_delta_as_empty(#[case] event: &str) {
    assert_eq!(
        mapped(event, FIXED_NOW),
        Some(json(
            r#"{"id":"stream-chunk","object":"chat.completion.chunk","created":1700000000,"model":"unknown","choices":[{"index":0,"delta":{}}]}"#
        ))
    );
}

#[rstest]
#[case::now_before_the_epoch(r#"{"orchestration_result":{"choices":[]}}"#, -1)]
#[case::event_before_the_epoch(
    r#"{"orchestration_result":{"created":-5,"choices":[]}}"#,
    FIXED_NOW
)]
fn rejects_a_negative_creation_time(#[case] event: &str, #[case] now: i64) {
    assert!(matches!(
        rejected(event, now),
        SapStreamError::Timestamp { .. }
    ));
}

#[rstest]
#[case::id(r#"{"request_id":5,"orchestration_result":{"choices":[]}}"#)]
#[case::model(r#"{"orchestration_result":{"model":5,"choices":[]}}"#)]
#[case::finish_reason(r#"{"orchestration_result":{"choices":[{"finish_reason":5}]}}"#)]
#[case::created_text(r#"{"orchestration_result":{"created":"5","choices":[]}}"#)]
#[case::created_boolean(r#"{"orchestration_result":{"created":true,"choices":[]}}"#)]
fn rejects_a_text_or_number_field_of_the_wrong_type(#[case] event: &str) {
    assert!(matches!(
        rejected(event, FIXED_NOW),
        SapStreamError::EventField(_)
    ));
}

#[test]
fn rejects_a_creation_time_beyond_the_integer_range() {
    assert!(matches!(
        rejected(
            r#"{"created":9223372036854775808,"orchestration_result":{"choices":[]}}"#,
            FIXED_NOW
        ),
        SapStreamError::EventField("created")
    ));
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::not_json("not json")]
#[case::empty_event("{}")]
#[case::data_without_space(r#"data:{"orchestration_result":{"id":"a"}}"#)]
fn skips_blank_and_unreadable_lines(#[case] line: &str) {
    let mut stream = new_stream();
    assert!(matches!(
        next(&mut stream, line, Mode::Sync).unwrap(),
        SapStep::Skipped
    ));
    assert!(!stream.done);
}

#[rstest]
#[case::prefixed(r#"data: {"orchestration_result":{"id":"a"}}"#)]
#[case::padded(r#"  data: {"orchestration_result":{"id":"a"}}  "#)]
#[case::two_spaces(r#"data:  {"orchestration_result":{"id":"a"}}"#)]
fn reads_a_line_after_the_data_prefix(#[case] line: &str) {
    let mut stream = new_stream();
    let step = next(&mut stream, line, Mode::Sync).unwrap();
    let SapStep::Chunk(chunk) = step else {
        panic!("expected a chunk");
    };
    assert_eq!(chunk.id, "a");
    assert!(!stream.done);
}

#[rstest]
#[case::prefixed("data: [DONE]")]
#[case::bare("[DONE]")]
#[case::padded("  data: [DONE]  ")]
fn ends_on_done(#[case] line: &str) {
    let mut stream = new_stream();
    assert!(matches!(
        next(&mut stream, line, Mode::Sync).unwrap(),
        SapStep::End
    ));
    assert!(stream.done);
}

#[test]
fn ends_on_done_in_async_mode_too() {
    let mut stream = new_stream();
    assert!(matches!(
        next(&mut stream, "data: [DONE]", Mode::Async).unwrap(),
        SapStep::End
    ));
    assert!(stream.done);
}

#[test]
fn does_not_end_on_a_lowercase_done() {
    let mut stream = new_stream();
    assert!(matches!(
        next(&mut stream, "data: [done]", Mode::Sync).unwrap(),
        SapStep::Skipped
    ));
    assert!(!stream.done);
}

#[rstest]
#[case::orchestration_sync(
    Mode::Sync,
    r#"data: {"orchestration_result":{"choices":[{"delta":{},"finish_reason":"stop"}]}}"#
)]
#[case::orchestration_async(
    Mode::Async,
    r#"data: {"orchestration_result":{"choices":[{"delta":{},"finish_reason":"stop"}]}}"#
)]
#[case::final_result_sync(
    Mode::Sync,
    r#"data: {"final_result":{"id":"f","created":7,"model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}}"#
)]
#[case::final_result_async(
    Mode::Async,
    r#"data: {"final_result":{"id":"f","created":7,"model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}}"#
)]
fn closes_after_a_finish_reason_chunk(#[case] mode: Mode, #[case] line: &str) {
    let mut stream = new_stream();
    let step = next(&mut stream, line, mode).unwrap();
    assert!(matches!(step, SapStep::Chunk(_)));
    assert!(stream.done);
    assert!(matches!(
        next(&mut stream, r#"data: {"error":"later"}"#, mode).unwrap(),
        SapStep::End
    ));
}

#[test]
fn keeps_reading_while_the_finish_reason_is_null() {
    let mut stream = new_stream();
    let first = next(
        &mut stream,
        r#"data: {"orchestration_result":{"id":"a","choices":[{"delta":{},"finish_reason":null}]}}"#,
        Mode::Sync,
    )
    .unwrap();
    assert!(matches!(first, SapStep::Chunk(_)));
    assert!(!stream.done);
    let second = next(
        &mut stream,
        r#"data: {"orchestration_result":{"id":"b","choices":[]}}"#,
        Mode::Sync,
    )
    .unwrap();
    let SapStep::Chunk(chunk) = second else {
        panic!("expected a chunk");
    };
    assert_eq!(chunk.id, "b");
}

#[test]
fn surfaces_an_error_event_in_sync_mode() {
    let mut stream = new_stream();
    let failure = next(&mut stream, r#"data: {"error":"boom"}"#, Mode::Sync).unwrap_err();
    let SapStreamError::Event { message } = failure else {
        panic!("expected an event error");
    };
    assert!(message.contains(r#"{"error":"boom"}"#));
    assert!(stream.done);
    assert!(matches!(
        next(
            &mut stream,
            r#"data: {"orchestration_result":{"id":"a"}}"#,
            Mode::Sync
        )
        .unwrap(),
        SapStep::End
    ));
}

#[test]
fn wraps_an_error_event_in_async_mode() {
    let mut stream = new_stream();
    let failure = next(&mut stream, r#"data: {"error":"boom"}"#, Mode::Async).unwrap_err();
    let SapStreamError::Orchestration(error) = failure else {
        panic!("expected an orchestration error");
    };
    assert_eq!(error.status_code, 502);
    assert!(error.message.contains(r#"{"error":"boom"}"#));
    assert!(stream.done);
}

#[rstest]
#[case::error_event(r#"data: {"error":"boom"}"#)]
#[case::bad_chunk(r#"data: {"final_result":{}}"#)]
fn wraps_a_stream_failure_as_a_502_in_async_mode(#[case] line: &str) {
    let mut stream = new_stream();
    let failure = next(&mut stream, line, Mode::Async).unwrap_err();
    assert!(matches!(
        failure,
        SapStreamError::Orchestration(error) if error.status_code == 502
    ));
    assert!(stream.done);
}

#[rstest]
#[case::bad_chunk(r#"data: {"final_result":{}}"#)]
#[case::bad_event(r#"data: {"orchestration_result":[1]}"#)]
fn keeps_the_original_error_in_sync_mode(#[case] line: &str) {
    let mut stream = new_stream();
    let failure = next(&mut stream, line, Mode::Sync).unwrap_err();
    assert!(!matches!(failure, SapStreamError::Orchestration(_)));
    assert!(stream.done);
}

#[test]
fn ends_when_the_lines_end() {
    let mut stream = new_stream();
    assert!(matches!(stream.finish(), SapStep::End));
    assert!(stream.done);
    assert!(matches!(
        next(
            &mut stream,
            r#"data: {"orchestration_result":{"id":"a"}}"#,
            Mode::Sync
        )
        .unwrap(),
        SapStep::End
    ));
}

#[test]
fn ends_quietly_on_a_read_failure_in_async_mode() {
    let mut stream = new_stream();
    assert!(matches!(
        stream.read_failed(ReadFailure, Mode::Async).unwrap(),
        SapStep::End
    ));
    assert!(stream.done);
}

#[test]
fn lets_a_read_failure_through_in_sync_mode() {
    let mut stream = new_stream();
    stream.read_failed(ReadFailure, Mode::Sync).unwrap_err();
    assert!(!stream.done);
}

#[test]
fn stops_immediately_once_done() {
    let mut stream = new_stream();
    assert!(matches!(
        next(&mut stream, "data: [DONE]", Mode::Sync).unwrap(),
        SapStep::End
    ));
    assert!(matches!(
        next(&mut stream, r#"data: {"error":"never read"}"#, Mode::Sync).unwrap(),
        SapStep::End
    ));
}

#[rstest]
#[case::absent(r#"{"config":{"modules":[]}}"#)]
#[case::null(r#"{"config":{"modules":[],"stream":null}}"#)]
fn adds_the_stream_flag_when_stream_is_absent_or_null(#[case] body: &str) {
    let mut parsed = json(body);
    add_stream_flag(&mut parsed).unwrap();
    assert_eq!(
        parsed,
        json(r#"{"config":{"modules":[],"stream":{"enabled":true}}}"#)
    );
}

#[test]
fn keeps_the_other_stream_keys() {
    let mut body =
        json(r#"{"config":{"stream":{"enabled":false,"chunk_size":5,"delimiters":["\n"]}}}"#);
    add_stream_flag(&mut body).unwrap();
    assert_eq!(
        body,
        json(r#"{"config":{"stream":{"enabled":true,"chunk_size":5,"delimiters":["\n"]}}}"#)
    );
}

#[test]
fn keeps_the_other_config_keys() {
    let mut body = json(r#"{"config":{"modules":[1],"spec_version":"v1"}}"#);
    add_stream_flag(&mut body).unwrap();
    assert_eq!(
        body,
        json(r#"{"config":{"modules":[1],"spec_version":"v1","stream":{"enabled":true}}}"#)
    );
}

#[rstest]
#[case::no_config("{}")]
#[case::null_config(r#"{"config":null}"#)]
#[case::list_config(r#"{"config":[]}"#)]
#[case::list_body("[1]")]
fn rejects_a_body_without_a_config_object(#[case] body: &str) {
    let mut parsed = json(body);
    assert!(matches!(
        add_stream_flag(&mut parsed).unwrap_err(),
        SapStreamError::ConfigBody
    ));
}

#[rstest]
#[case::text(r#"{"config":{"stream":"x"}}"#)]
#[case::list(r#"{"config":{"stream":[]}}"#)]
fn rejects_a_stream_that_is_not_an_object(#[case] body: &str) {
    let mut parsed = json(body);
    assert!(matches!(
        add_stream_flag(&mut parsed).unwrap_err(),
        SapStreamError::StreamBody
    ));
}

fn replay_line(text: &str) {
    let mut sync_line = new_stream();
    let _sync = sync_line.next_chunk(text, Mode::Sync);
    let mut async_line = new_stream();
    let _async = async_line.next_chunk(text, Mode::Async);
}

#[test]
fn replays_the_sap_stream_line_corpus() {
    let seen = fs::read_dir("fuzz/corpus/sap_stream_line")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| replay_line(&fs::read_to_string(path).unwrap()))
        .count();
    assert!(seen > 0_usize);
}
