#![cfg(test)]

use super::wire::{ResponsesAPIStreamingResponse, decode_responses_event_record};
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
#[case::created(r#"{"type":"response.created","item_id":"item_a","output_index":5,"content_index":7,"response":{"id":"resp_a","created_at":1,"output":[]}}"#, "ResponseCreated", false)]
#[case::in_progress(r#"{"type":"response.in_progress","item_id":"item_a","output_index":5,"content_index":7,"response":{"id":"resp_a","created_at":1,"output":[]}}"#, "ResponseInProgress", false)]
#[case::completed(r#"{"type":"response.completed","item_id":"item_a","output_index":5,"content_index":7,"response":{"id":"resp_a","created_at":1,"output":[]}}"#, "ResponseCompleted", false)]
#[case::failed(r#"{"type":"response.failed","item_id":"item_a","output_index":5,"content_index":7,"response":{"id":"resp_a","created_at":1,"output":[]}}"#, "ResponseFailed", false)]
#[case::incomplete(r#"{"type":"response.incomplete","item_id":"item_a","output_index":5,"content_index":7,"response":{"id":"resp_a","created_at":1,"output":[]}}"#, "ResponseIncomplete", false)]
#[case::reasoning_part_added(r#"{"type":"response.reasoning_summary_part.added","item_id":"item_a","output_index":5,"content_index":7,"part":{}}"#, "ResponsePartAdded", false)]
#[case::reasoning_delta(r#"{"type":"response.reasoning_summary_text.delta","item_id":"item_a","output_index":5,"content_index":7,"delta":"hi"}"#, "ReasoningSummaryTextDelta", false)]
#[case::reasoning_done(r#"{"type":"response.reasoning_summary_text.done","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":7,"text":"hi"}"#, "ReasoningSummaryTextDone", false)]
#[case::reasoning_part_done(r#"{"type":"response.reasoning_summary_part.done","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":8,"part":{}}"#, "ReasoningSummaryPartDone", false)]
#[case::output_item_added(r#"{"type":"response.output_item.added","item_id":"item_a","output_index":5,"content_index":7,"item":null}"#, "OutputItemAdded", false)]
#[case::output_item_done(r#"{"type":"response.output_item.done","item_id":"item_a","output_index":5,"content_index":7,"item":{"type":"message"}}"#, "OutputItemDone", false)]
#[case::content_added(r#"{"type":"response.content_part.added","item_id":"item_a","output_index":5,"content_index":7,"part":{}}"#, "ContentPartAdded", false)]
#[case::content_done(r#"{"type":"response.content_part.done","item_id":"item_a","output_index":5,"content_index":7,"part":{"type":"refusal","refusal":"no"}}"#, "ContentPartDone", false)]
#[case::output_text_delta(r#"{"type":"response.output_text.delta","item_id":"item_a","output_index":5,"content_index":7,"delta":"hi"}"#, "OutputTextDelta", false)]
#[case::annotation_added(r#"{"type":"response.output_text.annotation.added","item_id":"item_a","output_index":5,"content_index":7,"annotation_index":2,"annotation":{}}"#, "OutputTextAnnotationAdded", false)]
#[case::output_text_done(r#"{"type":"response.output_text.done","item_id":"item_a","output_index":5,"content_index":7,"text":"hi"}"#, "OutputTextDone", false)]
#[case::refusal_delta(r#"{"type":"response.refusal.delta","item_id":"item_a","output_index":5,"content_index":7,"delta":"no"}"#, "RefusalDelta", false)]
#[case::refusal_done(r#"{"type":"response.refusal.done","item_id":"item_a","output_index":5,"content_index":7,"refusal":"no"}"#, "RefusalDone", false)]
#[case::function_delta(r#"{"type":"response.function_call_arguments.delta","item_id":"item_a","output_index":5,"content_index":7,"delta":"open"}"#, "FunctionCallArgumentsDelta", false)]
#[case::function_done(r#"{"type":"response.function_call_arguments.done","item_id":"item_a","output_index":5,"content_index":7,"arguments":"empty"}"#, "FunctionCallArgumentsDone", false)]
#[case::custom_delta(r#"{"type":"response.custom_tool_call_input.delta","item_id":"item_a","output_index":5,"content_index":7,"delta":"patch"}"#, "Generic", true)]
#[case::custom_done(r#"{"type":"response.custom_tool_call_input.done","item_id":"item_a","output_index":5,"content_index":7,"input":"patch"}"#, "Generic", true)]
#[case::file_progress(r#"{"type":"response.file_search_call.in_progress","item_id":"item_a","output_index":5,"content_index":7}"#, "FileSearchCallInProgress", false)]
#[case::file_searching(r#"{"type":"response.file_search_call.searching","item_id":"item_a","output_index":5,"content_index":7}"#, "FileSearchCallSearching", false)]
#[case::file_done(r#"{"type":"response.file_search_call.completed","item_id":"item_a","output_index":5,"content_index":7}"#, "FileSearchCallCompleted", false)]
#[case::web_progress(r#"{"type":"response.web_search_call.in_progress","item_id":"item_a","output_index":5,"content_index":7}"#, "WebSearchCallInProgress", false)]
#[case::web_searching(r#"{"type":"response.web_search_call.searching","item_id":"item_a","output_index":5,"content_index":7}"#, "WebSearchCallSearching", false)]
#[case::web_done(r#"{"type":"response.web_search_call.completed","item_id":"item_a","output_index":5,"content_index":7}"#, "WebSearchCallCompleted", false)]
#[case::mcp_list_progress(r#"{"type":"response.mcp_list_tools.in_progress","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":4}"#, "McpListToolsInProgress", false)]
#[case::mcp_list_done(r#"{"type":"response.mcp_list_tools.completed","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":4}"#, "McpListToolsCompleted", false)]
#[case::mcp_list_failed(r#"{"type":"response.mcp_list_tools.failed","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":4}"#, "McpListToolsFailed", false)]
#[case::mcp_progress(r#"{"type":"response.mcp_call.in_progress","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":4}"#, "McpCallInProgress", false)]
#[case::mcp_delta(r#"{"type":"response.mcp_call_arguments.delta","item_id":"item_a","output_index":5,"content_index":7,"delta":"open","sequence_number":4}"#, "McpCallArgumentsDelta", false)]
#[case::mcp_args_done(r#"{"type":"response.mcp_call_arguments.done","item_id":"item_a","output_index":5,"content_index":7,"arguments":"empty","sequence_number":4}"#, "McpCallArgumentsDone", false)]
#[case::mcp_done(r#"{"type":"response.mcp_call.completed","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":4}"#, "McpCallCompleted", false)]
#[case::mcp_failed(r#"{"type":"response.mcp_call.failed","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":4}"#, "McpCallFailed", false)]
#[case::image_partial(r#"{"type":"image_generation.partial_image","item_id":"item_a","output_index":5,"content_index":7,"partial_image_index":1,"b64_json":"YQ=="}"#, "ImageGenerationPartialImage", false)]
#[case::shell_progress(r#"{"type":"response.shell_call.in_progress","item_id":"item_a","output_index":5,"content_index":7}"#, "Generic", true)]
#[case::shell_done(r#"{"type":"response.shell_call.completed","item_id":"item_a","output_index":5,"content_index":7}"#, "Generic", true)]
#[case::shell_output(r#"{"type":"response.shell_call_output.done","item_id":"item_a","output_index":5,"content_index":7}"#, "Generic", true)]
#[case::error_event(r#"{"type":"error","item_id":"item_a","output_index":5,"content_index":7,"sequence_number":9,"error":{"type":"invalid_request_error","code":"bad_request","message":"bad"}}"#, "Error", false)]
fn every_event_literal_decodes_typed_record(
    #[case] input: &str,
    #[case] expected_variant: &str,
    #[case] expected_generic: bool,
) {
    let parsed_input: Value = serde_json::from_str(input).unwrap();
    let event_type = parsed_input.get("type").and_then(Value::as_str).unwrap();
    let decoded =
        decode_responses_event_record(&serde_json::to_vec(&parsed_input).unwrap(), limits())
            .unwrap();
    match &decoded {
        ResponsesAPIStreamingResponse::Generic(event) => {
            assert!(expected_generic);
            assert_eq!(event.r#type, event_type);
        }
        ResponsesAPIStreamingResponse::Known(event) => {
            assert!(!expected_generic);
            assert!(format!("{:?}", event.as_ref()).starts_with(expected_variant));
        }
    }
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        encoded.get("type"),
        Some(&Value::String(event_type.to_owned()))
    );
    assert_eq!(encoded.get("item_id"), parsed_input.get("item_id"));
    assert_eq!(
        encoded.get("output_index"),
        parsed_input.get("output_index")
    );
    assert_eq!(
        encoded.get("content_index"),
        parsed_input.get("content_index")
    );
}

#[rstest]
#[case::shell_progress("response.shell_call.in_progress")]
#[case::shell_done("response.shell_call.completed")]
#[case::shell_output("response.shell_call_output.done")]
fn shell_events_keep_extra_fields(#[case] event: &str) {
    let input = format!("{{\"type\":\"{event}\",\"shell_debug\":\"kept\"}}");
    let decoded = decode_responses_event_record(input.as_bytes(), limits()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        encoded.get("shell_debug"),
        Some(&Value::String(String::from("kept")))
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
fn item_content_events() {
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

#[test]
fn output_item_events_preserve_provider_item_objects() {
    let input = br#"{"type":"response.output_item.done","output_index":2,"item":{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"hi","annotations":[],"logprobs":[]}]}}"#;
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    let original: Value = serde_json::from_slice(input).unwrap();
    assert_eq!(encoded.get("item"), original.get("item"));
    assert_eq!(encoded.get("output_index"), original.get("output_index"));
    assert_eq!(encoded.get("sequence_number"), Some(&Value::from(1_i64)));
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
fn reasoning_summary_text_done_requires_sequence_number() {
    let input = br#"{"type":"response.reasoning_summary_text.done","item_id":"i","output_index":0,"text":"done"}"#;
    assert!(matches!(
        decode_responses_event_record(input, limits()).unwrap_err(),
        super::wire::ResponsesEventDecodeError::Json(_)
    ));
}

#[rstest]
#[case::defaulted(br#"{"type":"response.reasoning_summary_text.done","item_id":"i","output_index":0,"sequence_number":7,"text":"done"}"#, 0)]
#[case::explicit(br#"{"type":"response.reasoning_summary_text.done","item_id":"i","output_index":0,"sequence_number":7,"summary_index":3,"text":"done"}"#, 3)]
fn reasoning_events(#[case] input: &[u8], #[case] expected_index: i64) {
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        encoded.get("summary_index"),
        Some(&Value::from(expected_index))
    );
}

#[test]
fn content_part_done_accepts_reasoning_text() {
    let input = br#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":1,"part":{"type":"reasoning_text","reasoning":"think"}}"#;
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        encoded.pointer("/part/type"),
        Some(&Value::String(String::from("reasoning_text")))
    );
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

#[rstest]
#[case::bytes_null(r#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":1,"part":{"type":"output_text","text":"hi","annotations":[],"logprobs":[{"token":"a","logprob":-1.5,"bytes":null} ]}}"#)]
#[case::top_logprobs_null(r#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":1,"part":{"type":"output_text","text":"hi","annotations":[],"logprobs":[{"token":"a","logprob":-1.5,"top_logprobs":null}]}}"#)]
#[case::top_bytes_null(r#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":1,"part":{"type":"output_text","text":"hi","annotations":[],"logprobs":[{"token":"a","logprob":-1.5,"top_logprobs":[{"token":"b","logprob":-2.0,"bytes":null}]}]}}"#)]
fn logprob_optional_lists_reject_null(#[case] input: &str) {
    assert!(matches!(
        decode_responses_event_record(input.as_bytes(), limits()).unwrap_err(),
        super::wire::ResponsesEventDecodeError::Json(_)
    ));
}

#[test]
fn logprob_bytes_accept_arbitrary_json_values() {
    let input = br#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":1,"part":{"type":"output_text","text":"hi","annotations":[],"logprobs":[{"token":"a","logprob":-1.5,"bytes":["x",true],"top_logprobs":[{"token":"b","logprob":-2.0,"bytes":["y"]}]}]}}"#;
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        encoded.pointer("/part/logprobs/0/bytes"),
        Some(&serde_json::json!(["x", true]))
    );
    assert_eq!(
        encoded.pointer("/part/logprobs/0/top_logprobs/0/bytes"),
        Some(&serde_json::json!(["y"]))
    );
}

#[test]
fn logprob_fields_round_trip() {
    let input = br#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":1,"part":{"type":"output_text","text":"hi","annotations":[],"logprobs":[{"token":"a","logprob":-1,"bytes":[97],"top_logprobs":[{"token":"b","logprob":-2}]}]}}"#;
    let input_value: Value = serde_json::from_slice(input).unwrap();
    let decoded = decode_responses_event_record(input, limits()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        encoded.pointer("/part/logprobs"),
        input_value.pointer("/part/logprobs")
    );
}

#[test]
fn error_and_generic() {
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
    let object = br#"{"type":"error","sequence_number":9,"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"too long","param":{"field":"input"}}}"#;
    let object_value =
        serde_json::to_value(decode_responses_event_record(object, limits()).unwrap()).unwrap();
    let expected_object = serde_json::from_str::<Value>(r#"{"field":"input"}"#).unwrap();
    assert_eq!(object_value.pointer("/error/param"), Some(&expected_object));
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

#[rstest]
#[case::created("response.created")]
#[case::in_progress("response.in_progress")]
#[case::completed("response.completed")]
#[case::failed("response.failed")]
#[case::incomplete("response.incomplete")]
fn lifecycle_events(#[case] event_type: &str) {
    let input = serde_json::from_str::<Value>(&format!(r#"{{"type":"{event_type}","response":{{"id":"resp_a","created_at":1,"output":[],"usage":{{"input_tokens":2,"output_tokens":1,"total_tokens":3}}}}}}"#)).unwrap();
    let decoded =
        decode_responses_event_record(&serde_json::to_vec(&input).unwrap(), limits()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(encoded.get("type"), input.get("type"));
    assert_eq!(
        encoded.pointer("/response/usage/input_tokens"),
        Some(&Value::from(2_i64))
    );
    assert_eq!(
        encoded.pointer("/response/usage/output_tokens"),
        Some(&Value::from(1_i64))
    );
    assert_eq!(
        encoded.pointer("/response/usage/total_tokens"),
        Some(&Value::from(3_i64))
    );
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
    assert_eq!(replayed.len(), 4);
}

#[rstest]
#[case::delta(r#"{"type":"response.reasoning_summary_text.delta","item_id":"i","output_index":0,"summary_index":0,"delta":""}"#, "summary_index", 0)]
#[case::text_done(r#"{"type":"response.reasoning_summary_text.done","item_id":"i","output_index":0,"sequence_number":7,"summary_index":0,"text":""}"#, "summary_index", 0)]
#[case::part_done(r#"{"type":"response.reasoning_summary_part.done","item_id":"i","output_index":0,"sequence_number":8,"summary_index":0,"part":{}}"#, "summary_index", 0)]
#[case::explicit_zero(
    r#"{"type":"response.output_item.done","output_index":2,"sequence_number":0,"item":{}}"#,
    "sequence_number",
    0
)]
fn explicit_indices_are_preserved(#[case] input: &str, #[case] field: &str, #[case] expected: i64) {
    let decoded = decode_responses_event_record(input.as_bytes(), limits()).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap().get(field),
        Some(&Value::from(expected))
    );
}

#[rstest]
#[case::summary_delta(r#"{"type":"response.reasoning_summary_text.delta","item_id":"i","output_index":0,"summary_index":null,"delta":""}"#)]
#[case::summary_text_done(r#"{"type":"response.reasoning_summary_text.done","item_id":"i","output_index":0,"sequence_number":7,"summary_index":null,"text":""}"#)]
#[case::summary_part_done(r#"{"type":"response.reasoning_summary_part.done","item_id":"i","output_index":0,"sequence_number":8,"summary_index":null,"part":{}}"#)]
#[case::output_item_done(r#"{"type":"response.output_item.done","output_index":2,"sequence_number":null,"item":{"type":"message"}}"#)]
fn null_indices_are_rejected(#[case] input: &str) {
    assert!(matches!(
        decode_responses_event_record(input.as_bytes(), limits()).unwrap_err(),
        super::wire::ResponsesEventDecodeError::Json(_)
    ));
}

#[rstest]
#[case::missing_type("{}")]
#[case::invalid_type(r#"{"type":1}"#)]
#[case::missing_nullable_item(r#"{"type":"response.output_item.added","output_index":2}"#)]
#[case::reasoning_part_sequence(
    r#"{"type":"response.reasoning_summary_part.done","item_id":"i","output_index":0,"part":{}}"#
)]
fn required_fields_are_not_defaulted(#[case] input: &str) {
    decode_responses_event_record(input.as_bytes(), limits()).unwrap_err();
}

#[rstest]
#[case::text_delta("response.output_text.delta", "delta", "hi")]
#[case::text_done("response.output_text.done", "text", "hi")]
#[case::refusal_delta("response.refusal.delta", "delta", "no")]
#[case::refusal_done("response.refusal.done", "refusal", "no")]
fn text_tool_events(#[case] event_type: &str, #[case] field: &str, #[case] payload: &str) {
    let input = serde_json::from_str::<Value>(&format!(r#"{{"type":"{event_type}","item_id":"i","output_index":0,"content_index":1,"{field}":"{payload}"}}"#)).unwrap();
    let decoded =
        decode_responses_event_record(&serde_json::to_vec(&input).unwrap(), limits()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), input);
}

#[rstest]
#[case::image(
    r#"{"type":"image_generation.partial_image","partial_image_index":1,"b64_json":"YQ=="}"#
)]
#[case::mcp_delta(r#"{"type":"response.mcp_call_arguments.delta","item_id":"tool_a","output_index":2,"sequence_number":4,"delta":"{"}"#)]
#[case::mcp_done(r#"{"type":"response.mcp_call_arguments.done","item_id":"tool_a","output_index":2,"sequence_number":4,"arguments":"{}"}"#)]
fn search_mcp_events(#[case] input: &str) {
    let decoded = decode_responses_event_record(input.as_bytes(), limits()).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::from_str::<Value>(input).unwrap()
    );
}

#[rstest]
#[case::completed("response.completed", true)]
#[case::incomplete("response.incomplete", true)]
#[case::failed("response.failed", true)]
#[case::created("response.created", false)]
#[case::in_progress("response.in_progress", false)]
fn terminal_events_only_accept_terminal_types(#[case] event_type: &str, #[case] terminal: bool) {
    let input = serde_json::from_str::<Value>(&format!(
        r#"{{"type":"{event_type}","response":{{"id":"resp_a","created_at":1,"output":[]}}}}"#
    ))
    .unwrap();
    match serde_json::from_value::<super::records::ResponsesTerminalEvent>(input.clone()) {
        Ok(record) => {
            assert!(terminal);
            assert_eq!(
                serde_json::to_value(record).unwrap().get("type"),
                input.get("type")
            );
        }
        Err(_) => assert!(!terminal),
    }
}

#[rstest]
#[case::list_progress("response.mcp_list_tools.in_progress")]
#[case::list_completed("response.mcp_list_tools.completed")]
#[case::list_failed("response.mcp_list_tools.failed")]
#[case::call_progress("response.mcp_call.in_progress")]
#[case::call_completed("response.mcp_call.completed")]
#[case::call_failed("response.mcp_call.failed")]
#[case::arguments_delta("response.mcp_call_arguments.delta")]
#[case::arguments_done("response.mcp_call_arguments.done")]
fn mcp_events_require_sequence_number(#[case] event_type: &str) {
    let input = serde_json::from_str::<Value>(&format!(r#"{{"type":"{event_type}","item_id":"tool_a","output_index":2,"delta":"{{","arguments":"{{}}"}}"#)).unwrap();
    let error =
        decode_responses_event_record(&serde_json::to_vec(&input).unwrap(), limits()).unwrap_err();
    assert!(error.to_string().contains("sequence_number"));
}
