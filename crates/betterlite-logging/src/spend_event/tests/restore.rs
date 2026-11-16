use super::support::{build_line, built, json, mapping, options, stamp};
use crate::spend_event::decode::decode_spend_event;
use crate::spend_event::input::{CachePresence, ResponseBody, ResponseObject};
use crate::spend_event::restore::{CallbackArgs, StreamingResponse, callback_args};
use crate::spend_event::usage::UsageError;

fn restored(kwargs: &str, response: ResponseObject) -> CallbackArgs {
    let mut call = options(false, CachePresence::On);
    call.response = response;
    let event = decode_spend_event(&build_line(kwargs, &call).unwrap()).unwrap();
    callback_args(event).unwrap()
}

fn mapping_response(fields: &str) -> ResponseObject {
    ResponseObject::Mapping(ResponseBody {
        fields: mapping(fields),
        tool_call_names: Vec::new(),
    })
}

#[test]
fn request_tools_come_back_as_minimal_function_declarations() {
    let kwargs = r#"{"tools": [{"name": "b"}, {"function": {"name": "a"}}, {"name": "b"}]}"#;
    let args = restored(kwargs, ResponseObject::Absent);
    let declared = |name: &str| format!(r#"{{"type":"function","function":{{"name":"{name}"}}}}"#);
    let expected = format!("[{},{},{}]", declared("b"), declared("a"), declared("b"));
    assert_eq!(serde_json::to_string(&args.kwargs.tools).unwrap(), expected);
}

#[test]
fn no_request_tools_restore_as_null() {
    let args = restored("{}", ResponseObject::Absent);
    assert_eq!(args.kwargs.tools, None);
}

#[test]
fn a_null_stream_key_comes_back_as_the_compact_response() {
    let response = mapping_response(r#"{"id": "r-1", "usage": {"total_tokens": 9}}"#);
    let args = restored(r#"{"complete_streaming_response": null}"#, response);
    let expected = r#"{"id":"r-1","usage":{"total_tokens":9}}"#;
    let StreamingResponse::Restored(streamed) = &args.kwargs.complete_streaming_response else {
        panic!("the stream key was recorded");
    };
    assert_eq!(serde_json::to_string(streamed).unwrap(), expected);
    assert_eq!(serde_json::to_string(&args.response_obj).unwrap(), expected);
}

#[test]
fn a_null_stream_key_with_no_response_restores_null() {
    let args = restored(
        r#"{"complete_streaming_response": null}"#,
        ResponseObject::Absent,
    );
    assert_eq!(
        args.kwargs.complete_streaming_response,
        StreamingResponse::Restored(None)
    );
}

#[test]
fn a_call_without_the_stream_key_restores_without_it() {
    let args = restored("{}", mapping_response(r#"{"id": "r-1"}"#));
    assert_eq!(
        args.kwargs.complete_streaming_response,
        StreamingResponse::Absent
    );
    assert_eq!(
        serde_json::to_string(&args.response_obj).unwrap(),
        r#"{"id":"r-1"}"#
    );
}

#[test]
fn usage_comes_back_with_its_counts_and_extra_fields() {
    let kwargs = r#"{"combined_usage_object": {"prompt_tokens": 3, "completion_tokens": null,
        "prompt_tokens_details": {"cached_tokens": 2}, "cost": 0.5}}"#;
    let args = restored(kwargs, ResponseObject::Absent);
    let usage = args.kwargs.combined_usage_object.unwrap();
    assert_eq!(usage.counts.prompt_tokens, 3);
    assert_eq!(usage.counts.completion_tokens, 0);
    assert_eq!(usage.counts.total_tokens, 0);
    assert_eq!(
        usage.rest.get("prompt_tokens_details"),
        Some(&json(r#"{"cached_tokens":2}"#))
    );
    assert_eq!(usage.rest.get("cost"), Some(&json("0.5")));
}

#[test]
fn billing_fields_survive_the_round_trip() {
    let kwargs = r#"{"call_type": "acompletion", "response_cost": 0.125, "cache_hit": true,
        "litellm_call_id": "c-1", "realtime_tools": [{"n": 1}], "realtime_tool_calls": [{"id": "x"}]}"#;
    let args = restored(kwargs, ResponseObject::Absent);
    let restored_kwargs = &args.kwargs;
    let cost = restored_kwargs.response_cost.map(|value| value.to_string());
    assert_eq!(cost.as_deref(), Some("0.125"));
    assert_eq!(restored_kwargs.cache_hit, Some(true));
    assert_eq!(restored_kwargs.call_type.as_deref(), Some("acompletion"));
    assert_eq!(restored_kwargs.litellm_call_id.as_deref(), Some("c-1"));
    assert_eq!(
        restored_kwargs.realtime_tools,
        Some(vec![json(r#"{"n":1}"#)])
    );
    assert_eq!(
        restored_kwargs.realtime_tool_calls,
        Some(vec![json(r#"{"id":"x"}"#)])
    );
    assert_eq!(args.start_time, stamp("2026-01-01T00:00:00Z"));
    assert_eq!(args.end_time, stamp("2026-01-01T00:00:01Z"));
}

#[test]
fn a_usage_count_that_is_not_an_integer_fails_the_restore() {
    let mut event = built("{}", &options(false, CachePresence::On));
    event.combined_usage = Some(mapping(r#"{"prompt_tokens": "many"}"#));
    let result = callback_args(event);
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
