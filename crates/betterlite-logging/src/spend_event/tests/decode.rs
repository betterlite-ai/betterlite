use rstest::rstest;

use super::support::{build_line, options};
use crate::spend_event::MAX_EVENT_BYTES;
use crate::spend_event::decode::decode_spend_event;
use crate::spend_event::error::SpendEventDecodeError;
use crate::spend_event::input::CachePresence;

const KWARGS: &str = r#"{
    "litellm_call_id": "call-1", "model": "gpt-4", "response_cost": 0.5,
    "completion_start_time": "2026-01-01T00:00:00.25+05:30",
    "combined_usage_object": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3},
    "tools": [{"name": "lookup"}], "complete_streaming_response": null
}"#;

fn valid_line() -> Vec<u8> {
    build_line(KWARGS, &options(false, CachePresence::On)).unwrap()
}

// The edit must hit the built line, or the case would test nothing
fn edited(from: &str, to: &str) -> Vec<u8> {
    let text = String::from_utf8(valid_line()).unwrap();
    assert!(text.contains(from), "{from}");
    text.replacen(from, to, 1).into_bytes()
}

#[test]
fn a_built_line_decodes_and_encodes_to_the_same_event() {
    let line = valid_line();
    let event = decode_spend_event(&line).unwrap();
    assert_eq!(event.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(
        event.response_cost.map(|cost| cost.to_string()).as_deref(),
        Some("0.5")
    );
    assert!(event.complete_streaming_response);
    assert_eq!(event.request_tool_names, ["lookup"]);
    let mut encoded = serde_json::to_vec(&event).unwrap();
    encoded.push(b'\n');
    assert_eq!(encoded, line);
}

#[test]
fn keeps_offsets_microseconds_and_naive_stamps() {
    let line = edited(
        r#""end_time":"2026-01-01T00:00:01Z""#,
        r#""end_time":"2026-02-03 04:05:06.000007""#,
    );
    let decoded = decode_spend_event(&line).unwrap();
    let text = serde_json::to_string(&decoded).unwrap();
    assert!(text.contains(r#""completion_start_time":"2026-01-01T00:00:00.250000+05:30""#));
    assert!(text.contains(r#""end_time":"2026-02-03T04:05:06.000007""#));
}

#[test]
fn trailing_whitespace_is_not_data() {
    let mut line = valid_line();
    line.extend_from_slice(b"  \n");
    decode_spend_event(&line).unwrap();
}

#[rstest]
#[case::unknown_version(r#""version":1"#, r#""version":2"#)]
#[case::version_as_text(r#""version":1"#, r#""version":"1""#)]
#[case::version_missing(r#""version":1,"#, "")]
#[case::malformed_date(
    r#""start_time":"2026-01-01T00:00:00Z""#,
    r#""start_time":"2026-13-45T99:00:00Z""#
)]
#[case::date_as_number(r#""end_time":"2026-01-01T00:00:01Z""#, r#""end_time":1"#)]
#[case::omitted_nullable_model(r#""model":"gpt-4","#, "")]
#[case::omitted_nullable_response(r#""response":null,"#, "")]
#[case::omitted_nullable_cost(r#""response_cost":0.5,"#, "")]
#[case::omitted_required_tool_names(r#","request_tool_names":["lookup"]"#, "")]
#[case::omitted_stream_flag(r#""complete_streaming_response":true,"#, "")]
#[case::unknown_field(r#""version":1,"#, r#""version":1,"extra":true,"#)]
#[case::wrong_type(r#""stream":null"#, r#""stream":"yes""#)]
#[case::bad_literal_in_response(
    r#""response":null"#,
    r#""response":{"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":null,"tool_calls":[]}}]}"#
)]
fn rejects_malformed_events(#[case] from: &str, #[case] to: &str) {
    let result = decode_spend_event(&edited(from, to));
    assert!(matches!(result, Err(SpendEventDecodeError::Event { .. })));
}

#[rstest]
#[case::second_event(b"{}".as_slice())]
#[case::garbage(b"x".as_slice())]
fn rejects_data_after_the_event(#[case] tail: &[u8]) {
    let mut line = valid_line();
    line.extend_from_slice(tail);
    let result = decode_spend_event(&line);
    assert!(matches!(result, Err(SpendEventDecodeError::Event { .. })));
}

#[test]
fn rejects_a_truncated_line() {
    let line = valid_line();
    decode_spend_event(&line[..40]).unwrap_err();
}

#[test]
fn rejects_a_line_over_the_size_limit_and_accepts_one_at_it() {
    let oversized = vec![b' '; MAX_EVENT_BYTES.saturating_add(1)];
    let result = decode_spend_event(&oversized);
    assert!(matches!(
        result,
        Err(SpendEventDecodeError::TooLarge { length, limit })
            if length == MAX_EVENT_BYTES.saturating_add(1) && limit == MAX_EVENT_BYTES
    ));
    let mut at_limit = valid_line();
    at_limit.resize(MAX_EVENT_BYTES, b' ');
    decode_spend_event(&at_limit).unwrap();
}

#[test]
fn the_line_limit_is_64_mebibytes() {
    assert_eq!(MAX_EVENT_BYTES, 0x0400_0000_usize);
}
