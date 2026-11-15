use rstest::rstest;
use serde_json::{Value, json};

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

fn with_field(key: &str, replacement: Option<Value>) -> Vec<u8> {
    let mut event: Value = serde_json::from_slice(&valid_line()).unwrap();
    let fields = event.as_object_mut().unwrap();
    if let Some(value) = replacement {
        fields.insert(key.to_owned(), value);
    } else {
        fields.remove(key);
    }
    serde_json::to_vec(&event).unwrap()
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
    let mut event: Value = serde_json::from_slice(&valid_line()).unwrap();
    event["end_time"] = json!("2026-02-03 04:05:06.000007");
    let decoded = decode_spend_event(&serde_json::to_vec(&event).unwrap()).unwrap();
    let text = serde_json::to_value(&decoded).unwrap();
    assert_eq!(
        text["completion_start_time"],
        "2026-01-01T00:00:00.250000+05:30"
    );
    assert_eq!(text["end_time"], "2026-02-03T04:05:06.000007");
}

#[test]
fn trailing_whitespace_is_not_data() {
    let mut line = valid_line();
    line.extend_from_slice(b"  \n");
    assert!(decode_spend_event(&line).is_ok());
}

#[rstest]
#[case::unknown_version("version", Some(json!(2_u8)))]
#[case::version_as_text("version", Some(json!("1")))]
#[case::version_missing("version", None)]
#[case::malformed_date("start_time", Some(json!("2026-13-45T99:00:00Z")))]
#[case::date_as_number("end_time", Some(json!(1_u8)))]
#[case::omitted_nullable_model("model", None)]
#[case::omitted_nullable_response("response", None)]
#[case::omitted_nullable_cost("response_cost", None)]
#[case::omitted_required_tool_names("request_tool_names", None)]
#[case::omitted_stream_flag("complete_streaming_response", None)]
#[case::unknown_field("extra", Some(json!(true)))]
#[case::wrong_type("stream", Some(json!("yes")))]
#[case::bad_literal_in_response("response", Some(json!({"choices": [{"index": 0_u8, "finish_reason": "stop", "message": {"role": "assistant", "content": null, "tool_calls": []}}]})))]
fn rejects_malformed_events(#[case] key: &str, #[case] replacement: Option<Value>) {
    let result = decode_spend_event(&with_field(key, replacement));
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
    let cut = line.get(..40).unwrap();
    assert!(decode_spend_event(cut).is_err());
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
    assert!(decode_spend_event(&at_limit).is_ok());
}

#[test]
fn the_line_limit_is_64_mebibytes() {
    assert_eq!(MAX_EVENT_BYTES, 67_108_864_usize);
}
