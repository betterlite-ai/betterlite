use serde_json::{Value, json};

use super::support::{build_line, mapping, options};
use crate::spend_event::decode::decode_spend_event;
use crate::spend_event::input::{CachePresence, ResponseBody, ResponseObject};
use crate::spend_event::restore::{StreamingResponse, callback_args};
use crate::spend_event::usage::UsageError;

fn restored(kwargs: &str, response: ResponseObject) -> Value {
    let mut call = options(false, CachePresence::On);
    call.response = response;
    let event = decode_spend_event(&build_line(kwargs, &call).unwrap()).unwrap();
    serde_json::to_value(callback_args(event).unwrap()).unwrap()
}

fn mapping_response(fields: &str, names: &[&str]) -> ResponseObject {
    ResponseObject::Mapping(ResponseBody {
        fields: mapping(fields),
        tool_call_names: names.iter().map(|name| (*name).to_owned()).collect(),
    })
}

#[test]
fn request_tools_come_back_as_minimal_function_declarations() {
    let kwargs = r#"{"tools": [{"name": "b"}, {"function": {"name": "a"}}, {"name": "b"}]}"#;
    let args = restored(kwargs, ResponseObject::Absent);
    let declared = |name: &str| json!({"type": "function", "function": {"name": name}});
    assert_eq!(
        args["kwargs"]["tools"],
        json!([declared("b"), declared("a"), declared("b")])
    );
}

#[test]
fn no_request_tools_restore_as_null() {
    let args = restored("{}", ResponseObject::Absent);
    assert_eq!(args["kwargs"]["tools"], Value::Null);
}

#[test]
fn a_null_stream_key_comes_back_as_the_compact_response() {
    let response = mapping_response(r#"{"id": "r-1", "usage": {"total_tokens": 9}}"#, &[]);
    let args = restored(r#"{"complete_streaming_response": null}"#, response);
    let expected = json!({"id": "r-1", "usage": {"total_tokens": 9_u8}});
    assert_eq!(args["kwargs"]["complete_streaming_response"], expected);
    assert_eq!(args["response_obj"], expected);
}

#[test]
fn a_null_stream_key_with_no_response_restores_null() {
    let args = restored(
        r#"{"complete_streaming_response": null}"#,
        ResponseObject::Absent,
    );
    let kwargs = args["kwargs"].as_object().unwrap();
    assert_eq!(kwargs["complete_streaming_response"], Value::Null);
    assert!(kwargs.contains_key("complete_streaming_response"));
}

#[test]
fn a_call_without_the_stream_key_restores_without_it() {
    let args = restored("{}", mapping_response(r#"{"id": "r-1"}"#, &[]));
    assert!(
        !args["kwargs"]
            .as_object()
            .unwrap()
            .contains_key("complete_streaming_response")
    );
    assert_eq!(args["response_obj"], json!({"id": "r-1"}));
}

#[test]
fn usage_comes_back_with_its_counts_and_extra_fields() {
    let kwargs = r#"{"combined_usage_object": {"prompt_tokens": 3, "completion_tokens": null,
        "prompt_tokens_details": {"cached_tokens": 2}, "cost": 0.5}}"#;
    let args = restored(kwargs, ResponseObject::Absent);
    let usage = &args["kwargs"]["combined_usage_object"];
    assert_eq!(usage["prompt_tokens"], 3_u8);
    assert_eq!(usage["completion_tokens"], 0_u8);
    assert_eq!(usage["total_tokens"], 0_u8);
    assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 2_u8);
    assert_eq!(usage["cost"].to_string(), "0.5");
}

#[test]
fn billing_fields_survive_the_round_trip() {
    let kwargs = r#"{"call_type": "acompletion", "response_cost": 0.125, "cache_hit": true,
        "litellm_call_id": "c-1", "realtime_tools": [{"n": 1}], "realtime_tool_calls": [{"id": "x"}]}"#;
    let args = restored(kwargs, ResponseObject::Absent);
    let restored_kwargs = &args["kwargs"];
    assert_eq!(restored_kwargs["response_cost"].to_string(), "0.125");
    assert_eq!(restored_kwargs["cache_hit"], true);
    assert_eq!(restored_kwargs["call_type"], "acompletion");
    assert_eq!(restored_kwargs["realtime_tools"][0]["n"], 1_u8);
    assert_eq!(restored_kwargs["realtime_tool_calls"][0]["id"], "x");
    assert_eq!(args["start_time"], "2026-01-01T00:00:00Z");
    assert_eq!(args["end_time"], "2026-01-01T00:00:01Z");
}

#[test]
fn a_usage_count_that_is_not_an_integer_fails_the_restore() {
    let line = build_line("{}", &options(false, CachePresence::On)).unwrap();
    let mut event: Value = serde_json::from_slice(&line).unwrap();
    event["combined_usage"] = json!({"prompt_tokens": "many"});
    let decoded = decode_spend_event(&serde_json::to_vec(&event).unwrap()).unwrap();
    let result = callback_args(decoded);
    assert!(matches!(
        result,
        Err(UsageError::Count {
            field: "prompt_tokens"
        })
    ));
}

#[test]
fn the_streaming_marker_distinguishes_absent_from_a_null_response() {
    assert!(StreamingResponse::Absent.is_absent());
    assert!(!StreamingResponse::Restored(None).is_absent());
}
