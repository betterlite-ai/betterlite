#![cfg(test)]

use rstest::rstest;
use serde_json::{Value, json};

use super::DeleteResponseResult;
use super::builders::{build_interpreter_logs, build_web_search_call};
use super::wire::{WireDecodeLimits, decode_responses_shapes};

#[test]
fn web_search_sources_filters_invalid_rows_and_failure_marker() {
    let built = build_web_search_call(
        "t",
        &json!({"query":"cats"}),
        &json!({"content":[{"type":"web_search_result","url":"https://a.test"},{"type":"ignored","url":"https://b.test"},{"type":"web_search_result","url":3}]}),
        None,
    );
    assert_eq!(
        serde_json::to_value(built).unwrap(),
        json!({"id":"ws_t","type":"web_search_call","status":"completed","action":{"type":"search","query":"cats","queries":["cats"],"sources":[{"type":"url","url":"https://a.test"}]}})
    );
    let failed = build_web_search_call(
        "t",
        &Value::Null,
        &json!({"content":{"type":"web_search_tool_result_error"}}),
        Some("searching"),
    );
    assert_eq!(failed.status, "searching");
    assert_eq!(
        build_web_search_call(
            "t",
            &Value::Null,
            &json!({"content":{"type":"web_search_tool_result_error"}}),
            None
        )
        .status,
        "failed"
    );
    let missing_query =
        build_web_search_call("t", &json!({"query":4}), &json!({"content":"text"}), None);
    assert_eq!(missing_query.action.query, "");
    assert!(missing_query.action.queries.is_empty());
    assert!(missing_query.action.sources.is_empty());
}

#[test]
fn interpreter_logs_concatenate_and_drop_empty_values() {
    let built = build_interpreter_logs(&json!({"stdout":"out","stderr":"err"}));
    assert_eq!(
        serde_json::to_value(built).unwrap(),
        json!([{"type":"logs","logs":"outSTDERR: err"}])
    );
    assert_eq!(
        build_interpreter_logs(&json!({"stderr":"err"})).unwrap()[0].logs,
        "STDERR: err"
    );
    assert!(build_interpreter_logs(&json!({})).is_none());
    assert!(build_interpreter_logs(&json!({"stdout":"","stderr":""})).is_none());
    assert!(build_interpreter_logs(&json!([])).is_none());
}

#[test]
fn nullable_records_require_nullable_fields_and_omit_private_fields() {
    let deleted: DeleteResponseResult =
        serde_json::from_value(json!({"id":null,"object":null,"deleted":null})).unwrap();
    assert_eq!(
        serde_json::to_value(deleted).unwrap(),
        json!({"id":null,"object":null,"deleted":null})
    );
    let missing = serde_json::from_value::<DeleteResponseResult>(json!({"id":null,"object":null}));
    assert!(
        missing.is_err(),
        "missing nullable value decoded as {missing:?}"
    );
}

#[test]
fn custom_and_generic_output_items_keep_required_and_default_fields() {
    let custom: super::CustomToolCallOutputItem = serde_json::from_value(
        json!({"type":"custom_tool_call","call_id":"c1","name":"apply_patch","input":"diff"}),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(custom).unwrap(),
        json!({"type":"custom_tool_call","call_id":"c1","id":null,"name":"apply_patch","input":"diff","status":null})
    );
    assert!(
        serde_json::from_value::<super::CustomToolCallOutputItem>(
            json!({"type":"custom_tool_call","call_id":"c1","name":"apply_patch"})
        )
        .is_err()
    );
    let generic: super::GenericResponseOutputItem = serde_json::from_value(
        json!({"type":"message","id":"m1","status":"completed","role":"assistant","content":[]}),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(generic).unwrap()["phase"], Value::Null);
}

#[rstest]
#[case::in_progress("in_progress")]
#[case::completed("completed")]
#[case::incomplete("incomplete")]
#[case::failed("failed")]
fn image_and_interpreter_call_status_values(#[case] status: &str) {
    assert!(
        serde_json::from_value::<super::OutputImageGenerationCall>(
            json!({"type":"image_generation_call","id":"i","status":status,"result":null})
        )
        .is_ok()
    );
    assert!(serde_json::from_value::<super::OutputCodeInterpreterCall>(json!({"type":"code_interpreter_call","id":"i","status":status,"code":null,"container_id":null,"outputs":null})).is_ok());
}

#[test]
fn phase_and_function_status_reject_unsupported_literals() {
    assert!(serde_json::from_value::<super::GenericResponseOutputItem>(json!({"type":"message","id":"m1","status":"completed","role":"assistant","content":[],"phase":"analysis"})).is_err());
    assert!(serde_json::from_value::<super::OutputFunctionToolCall>(json!({"arguments":"{}","call_id":"c","name":"f","type":"function_call","id":"i","status":"failed"})).is_err());
}

#[test]
fn websocket_defaults_use_fill_request_override_precedence() {
    let defaults = super::WsRequestDefaults {
        fill_missing: json!({"model":"a","temperature":1})
            .as_object()
            .unwrap()
            .clone(),
        overrides: json!({"model":"c"}).as_object().unwrap().clone(),
    };
    let request = json!({"model":"b","input":"hi"})
        .as_object()
        .unwrap()
        .clone();
    let expected = json!({"model":"c","temperature":1,"input":"hi"});
    assert_eq!(
        defaults.merged_into(&request),
        expected.as_object().unwrap().clone()
    );
    assert_eq!(request.get("model").unwrap(), "b");
}

#[test]
fn decoder_applies_byte_and_item_limits() {
    let limits = WireDecodeLimits {
        max_json_bytes: 100,
        max_event_bytes: 100,
        max_items: 3,
    };
    assert!(decode_responses_shapes(br#"{"id":"x"}"#, limits).is_ok());
    assert!(decode_responses_shapes(br#"{"a":[1,2,3]}"#, limits).is_err());
    let event_limited = WireDecodeLimits {
        max_json_bytes: 100,
        max_event_bytes: 1,
        max_items: 100,
    };
    assert!(decode_responses_shapes(b"{}", event_limited).is_err());
    let json_limited = WireDecodeLimits {
        max_json_bytes: 1,
        max_event_bytes: 100,
        max_items: 100,
    };
    assert!(decode_responses_shapes(b"{}", json_limited).is_err());
}

#[test]
fn replays_corpus() {
    let limits = WireDecodeLimits {
        max_json_bytes: 1_000_000,
        max_event_bytes: 1_000_000,
        max_items: 10_000,
    };
    std::fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/responses_shapes"
    ))
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .for_each(|path| {
        let bytes = std::fs::read(path).unwrap();
        let _decoded = decode_responses_shapes(&bytes, limits);
    });
}
