#![cfg(test)]

use super::wire::{
    ResponsesAPIStreamEvents, ResponsesAPIStreamingResponse, decode_responses_event_record,
};
use crate::api::responses_shapes::wire::WireDecodeLimits;
use rstest::rstest;
use serde_json::Value;
use std::fs;
use std::path::Path;

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0001_0000,
        max_event_bytes: 0x0001_0000,
        max_items: 4096,
    }
}

#[rstest]
#[case::created("response.created")]
#[case::in_progress("response.in_progress")]
#[case::completed("response.completed")]
#[case::failed("response.failed")]
#[case::incomplete("response.incomplete")]
#[case::reasoning_part_added("response.reasoning_summary_part.added")]
#[case::reasoning_delta("response.reasoning_summary_text.delta")]
#[case::reasoning_done("response.reasoning_summary_text.done")]
#[case::reasoning_part_done("response.reasoning_summary_part.done")]
#[case::output_item_added("response.output_item.added")]
#[case::output_item_done("response.output_item.done")]
#[case::content_added("response.content_part.added")]
#[case::content_done("response.content_part.done")]
#[case::output_text_delta("response.output_text.delta")]
#[case::annotation_added("response.output_text.annotation.added")]
#[case::output_text_done("response.output_text.done")]
#[case::refusal_delta("response.refusal.delta")]
#[case::refusal_done("response.refusal.done")]
#[case::function_delta("response.function_call_arguments.delta")]
#[case::function_done("response.function_call_arguments.done")]
#[case::custom_delta("response.custom_tool_call_input.delta")]
#[case::custom_done("response.custom_tool_call_input.done")]
#[case::file_progress("response.file_search_call.in_progress")]
#[case::file_searching("response.file_search_call.searching")]
#[case::file_done("response.file_search_call.completed")]
#[case::web_progress("response.web_search_call.in_progress")]
#[case::web_searching("response.web_search_call.searching")]
#[case::web_done("response.web_search_call.completed")]
#[case::mcp_list_progress("response.mcp_list_tools.in_progress")]
#[case::mcp_list_done("response.mcp_list_tools.completed")]
#[case::mcp_list_failed("response.mcp_list_tools.failed")]
#[case::mcp_progress("response.mcp_call.in_progress")]
#[case::mcp_delta("response.mcp_call_arguments.delta")]
#[case::mcp_args_done("response.mcp_call_arguments.done")]
#[case::mcp_done("response.mcp_call.completed")]
#[case::mcp_failed("response.mcp_call.failed")]
#[case::image_partial("image_generation.partial_image")]
#[case::shell_progress("response.shell_call.in_progress")]
#[case::shell_done("response.shell_call.completed")]
#[case::shell_output("response.shell_call_output.done")]
#[case::error_event("error")]
fn every_event_literal_deserializes(#[case] literal: &str) {
    let event: ResponsesAPIStreamEvents =
        serde_json::from_value(Value::String(literal.to_owned())).unwrap();
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        Value::String(literal.to_owned())
    );
}

#[test]
fn generic_event_keeps_unknown_and_protected_fields() {
    let record =
        decode_responses_event_record(br#"{"type":"vendor.future","model_debug":7}"#, limits())
            .unwrap();
    assert!(
        matches!(record, ResponsesAPIStreamingResponse::Generic(event) if event.extra.contains_key("model_debug"))
    );
}

#[test]
fn event_defaults_and_required_nullable_fields_are_preserved() {
    let added = decode_responses_event_record(
        br#"{"type":"response.output_item.added","output_index":2,"item":null}"#,
        limits(),
    )
    .unwrap();
    assert!(matches!(
        added,
        ResponsesAPIStreamingResponse::Known(event)
            if matches!(event.as_ref(), super::records::KnownResponseEvent::OutputItemAdded(record) if matches!(record.item, super::event_types::RequiredNullable::Null))
    ));
    let done = decode_responses_event_record(
        br#"{"type":"response.output_item.done","output_index":2,"item":{"type":"message"}}"#,
        limits(),
    )
    .unwrap();
    assert!(matches!(
        done,
        ResponsesAPIStreamingResponse::Known(event)
            if matches!(event.as_ref(), super::records::KnownResponseEvent::OutputItemDone(record) if record.sequence_number == 1)
    ));
}

#[rstest]
#[case::added_empty("response.reasoning_summary_part.added", "{}")]
#[case::added_other_type("response.reasoning_summary_part.added", "{\"type\":\"other\"}")]
#[case::done_empty("response.reasoning_summary_part.done", "{}")]
#[case::done_other_type("response.reasoning_summary_part.done", "{\"type\":\"other\"}")]
fn reasoning_summary_part_accepts_any_object(#[case] event: &str, #[case] part: &str) {
    let input = format!(
        "{{\"type\":\"{event}\",\"item_id\":\"rs_a\",\"output_index\":1,\"sequence_number\":8,\"part\":{part}}}"
    );
    decode_responses_event_record(input.as_bytes(), limits()).unwrap();
}

#[test]
fn event_byte_limit_is_enforced() {
    let small = WireDecodeLimits {
        max_json_bytes: 10,
        max_event_bytes: 10,
        max_items: 100,
    };
    decode_responses_event_record(br#"{"type":"error"}"#, small).unwrap_err();
}

#[test]
fn json_byte_limit_rejects_only_oversized_records() {
    let input = br#"{"type":"vendor.future"}"#;
    let too_small = WireDecodeLimits {
        max_json_bytes: input.len().saturating_sub(1),
        max_event_bytes: input.len().saturating_add(1),
        max_items: 100,
    };
    assert!(matches!(
        decode_responses_event_record(input, too_small),
        Err(super::wire::ResponsesEventDecodeError::JsonTooLarge)
    ));
    let exact = WireDecodeLimits {
        max_json_bytes: input.len(),
        max_event_bytes: input.len().saturating_add(1),
        max_items: 100,
    };
    decode_responses_event_record(input, exact).unwrap();
}

#[test]
fn event_byte_limit_rejects_only_oversized_records() {
    let input = br#"{"type":"vendor.future"}"#;
    let too_small = WireDecodeLimits {
        max_json_bytes: input.len().saturating_add(1),
        max_event_bytes: input.len().saturating_sub(1),
        max_items: 100,
    };
    assert!(matches!(
        decode_responses_event_record(input, too_small),
        Err(super::wire::ResponsesEventDecodeError::JsonTooLarge)
    ));
    let exact = WireDecodeLimits {
        max_json_bytes: input.len().saturating_add(1),
        max_event_bytes: input.len(),
        max_items: 100,
    };
    decode_responses_event_record(input, exact).unwrap();
}

#[test]
fn missing_required_field_is_rejected() {
    let input = br#"{"type":"response.mcp_call.completed","sequence_number":4,"item_id":"tool_a"}"#;
    let error = decode_responses_event_record(input, limits()).unwrap_err();
    assert!(matches!(
        error,
        super::wire::ResponsesEventDecodeError::Json(_)
    ));
}

#[test]
fn output_text_content_part_requires_nullable_logprobs_field() {
    let missing = br#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":1,"part":{"type":"output_text","text":"hi","annotations":[]}}"#;
    assert!(matches!(
        decode_responses_event_record(missing, limits()).unwrap_err(),
        super::wire::ResponsesEventDecodeError::Json(_)
    ));
    let explicit_null = br#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":1,"part":{"type":"output_text","text":"hi","annotations":[],"logprobs":null}}"#;
    decode_responses_event_record(explicit_null, limits()).unwrap();
}

#[test]
fn error_event_param_defaults_to_null_and_keeps_explicit_value() {
    let input = br#"{"type":"error","sequence_number":9,"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"too long"}}"#;
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    let value = serde_json::to_value(decoded).unwrap();
    assert_eq!(value.pointer("/error/param"), Some(&Value::Null));
    let explicit = br#"{"type":"error","sequence_number":9,"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"too long","param":"input"}}"#;
    let explicit_decoded = decode_responses_event_record(explicit, limits()).unwrap();
    let explicit_value = serde_json::to_value(explicit_decoded).unwrap();
    assert_eq!(
        explicit_value.pointer("/error/param"),
        Some(&Value::String(String::from("input")))
    );
}

#[test]
fn custom_tool_event_uses_generic_record_path() {
    let input =
        br#"{"type":"response.custom_tool_call_input.delta","item_id":"i","delta":"patch"}"#;
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    assert!(matches!(
        decoded,
        ResponsesAPIStreamingResponse::Generic(event)
            if event.r#type == "response.custom_tool_call_input.delta"
    ));
}

#[test]
fn completed_event_hidden_params_are_not_serialized() {
    let input = br#"{"type":"response.completed","response":{"id":"resp_a","created_at":1,"output":[],"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}}"#;
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert!(encoded.get("_hidden_params").is_none());
}

#[test]
fn replays_corpus() {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_event_records");
    let replayed: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(path).unwrap();
            decode_responses_event_record(&bytes, limits())
        })
        .collect();
    assert_eq!(replayed.len(), 2);
}
