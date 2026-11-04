#![cfg(test)]

use super::wire::{
    ResponsesAPIStreamEvents, ResponsesAPIStreamingResponse, WireDecodeLimits,
    decode_responses_event_record,
};
use rstest::rstest;
use serde_json::Value;
use std::fs;
use std::path::Path;

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 65536,
        max_event_bytes: 65536,
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
        matches!(record, ResponsesAPIStreamingResponse::Generic { fields, .. } if fields.get("model_debug").is_some())
    );
}

#[test]
fn event_defaults_and_required_nullable_fields_are_preserved() {
    let added = decode_responses_event_record(
        br#"{"type":"response.output_item.added","output_index":2,"item":null}"#,
        limits(),
    )
    .unwrap();
    assert!(
        matches!(added, ResponsesAPIStreamingResponse::Known { fields, .. } if fields.get("item") == Some(&betterlite_models::decisions::Json::Null))
    );
    let done = decode_responses_event_record(
        br#"{"type":"response.output_item.done","output_index":2,"item":{"type":"message"}}"#,
        limits(),
    )
    .unwrap();
    assert!(
        matches!(done, ResponsesAPIStreamingResponse::Known { fields, .. } if fields.get("sequence_number") == Some(&betterlite_models::decisions::Json::Number(1.into())))
    );
}

#[test]
fn event_byte_limit_is_enforced() {
    let small = WireDecodeLimits {
        max_json_bytes: 10,
        max_event_bytes: 10,
        max_items: 100,
    };
    assert!(decode_responses_event_record(br#"{"type":"error"}"#, small).is_err());
}

#[test]
fn completed_event_hidden_params_are_not_serialized() {
    let input = br#"{"type":"response.completed","response":{"id":"resp_a","created_at":1,"output":[],"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}}"#;
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    let encoded = serde_json::to_value(match decoded {
        ResponsesAPIStreamingResponse::Known { fields, .. } => fields,
        ResponsesAPIStreamingResponse::Generic { fields, .. } => fields,
    })
    .unwrap();
    assert!(encoded.get("_hidden_params").is_none());
}

#[test]
fn replays_corpus() {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_event_records");
    fs::read_dir(directory).unwrap().for_each(|entry| {
        let path = entry.unwrap().path();
        let bytes = fs::read(path).unwrap();
        let _decoded = decode_responses_event_record(&bytes, limits());
    });
}
