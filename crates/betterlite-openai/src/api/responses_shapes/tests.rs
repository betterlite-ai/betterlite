#![cfg(test)]

use rstest::rstest;
use serde_json::json;
use serde_json::value::RawValue;
use std::fs;

use super::DeleteResponseResult;
use super::builders::{WebSearchStatus, build_interpreter_logs, build_web_search_call};
use super::wire::{WireDecodeLimits, decode_responses_shapes};

#[test]
fn web_search_sources() {
    let built = build_web_search_call(
        "t",
        &serde_json::from_value(json!({"query":"cats"})).unwrap(),
        &serde_json::from_value(json!({"content":[{"type":"web_search_result","url":"https://a.test"},{"type":"ignored","url":"https://b.test"},{"type":"web_search_result","url":3}]})).unwrap(),
        None,
    );
    assert_eq!(
        serde_json::to_value(built).unwrap(),
        json!({"id":"ws_t","type":"web_search_call","status":"completed","action":{"type":"search","query":"cats","queries":["cats"],"sources":[{"type":"url","url":"https://a.test"}]}})
    );
    let failed = build_web_search_call(
        "t",
        &serde_json::from_str("null").unwrap(),
        &serde_json::from_value(json!({"content":{"type":"web_search_tool_result_error"}}))
            .unwrap(),
        Some(WebSearchStatus::Searching),
    );
    assert_eq!(failed.status, WebSearchStatus::Searching);
    assert_eq!(
        build_web_search_call(
            "t",
            &serde_json::from_str("null").unwrap(),
            &serde_json::from_value(json!({"content":{"type":"web_search_tool_result_error"}}))
                .unwrap(),
            None
        )
        .status,
        WebSearchStatus::Failed
    );
    let missing_query = build_web_search_call(
        "t",
        &serde_json::from_value(json!({"query":4})).unwrap(),
        &serde_json::from_value(json!({"content":"text"})).unwrap(),
        None,
    );
    assert_eq!(missing_query.action.query, "");
    assert!(missing_query.action.queries.is_empty());
    assert!(missing_query.action.sources.is_empty());
}

#[test]
fn interpreter_logs() {
    let built = build_interpreter_logs(
        &serde_json::from_value(json!({"stdout":"out","stderr":"err"})).unwrap(),
    );
    assert_eq!(
        serde_json::to_value(built).unwrap(),
        json!([{"type":"logs","logs":"outSTDERR: err"}])
    );
    assert_eq!(
        build_interpreter_logs(&serde_json::from_value(json!({"stderr":"err"})).unwrap()).unwrap()
            [0]
        .logs,
        "STDERR: err"
    );
    assert!(build_interpreter_logs(&serde_json::from_value(json!({})).unwrap()).is_none());
    assert!(
        build_interpreter_logs(&serde_json::from_value(json!({"stdout":"","stderr":""})).unwrap())
            .is_none()
    );
    assert!(build_interpreter_logs(&serde_json::from_value(json!([])).unwrap()).is_none());
}

#[test]
fn nullable_records() {
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
    assert_eq!(serde_json::to_value(generic).unwrap()["phase"], json!(null));
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
fn websocket_defaults() {
    let defaults: super::websocket::WsRequestDefaults = serde_json::from_value(json!({
        "fill_missing":{"model":"a","temperature":1},"overrides":{"model":"c"}
    }))
    .unwrap();
    let before = serde_json::to_value(&defaults).unwrap();
    let request = serde_json::from_str(r#"{"model":"b","input":"hi"}"#).unwrap();
    assert_eq!(
        serde_json::to_value(defaults.merged_into(&request)).unwrap(),
        json!({"model":"c","temperature":1,"input":"hi"})
    );
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"model":"b","input":"hi"})
    );
    assert_eq!(serde_json::to_value(&defaults).unwrap(), before);
}

#[test]
fn decoder_applies_byte_and_item_limits() {
    let limits = WireDecodeLimits {
        max_json_bytes: 100,
        max_event_bytes: 100,
        max_items: 3,
    };
    assert!(
        decode_responses_shapes::<super::OutputText>(
            br#"{"type":null,"text":null,"annotations":null}"#,
            limits
        )
        .is_ok()
    );
    assert!(decode_responses_shapes::<super::OutputText>(br#"{"a":[1,2,3]}"#, limits).is_err());
    let event_limited = WireDecodeLimits {
        max_json_bytes: 100,
        max_event_bytes: 1,
        max_items: 100,
    };
    assert!(matches!(
        decode_responses_shapes::<super::OutputText>(b"{}", event_limited),
        Err(super::wire::ShapeDecodeError::JsonTooLarge)
    ));
    let json_limited = WireDecodeLimits {
        max_json_bytes: 1,
        max_event_bytes: 100,
        max_items: 100,
    };
    assert!(matches!(
        decode_responses_shapes::<super::OutputText>(b"{}", json_limited),
        Err(super::wire::ShapeDecodeError::JsonTooLarge)
    ));
}

#[test]
fn replays_corpus() {
    let limits = WireDecodeLimits {
        max_json_bytes: 1_000_000,
        max_event_bytes: 1_000_000,
        max_items: 10_000,
    };
    fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/responses_shapes"
    ))
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .for_each(|path| {
        let bytes = fs::read(path).unwrap();
        let _deletion = decode_responses_shapes::<DeleteResponseResult>(&bytes, limits);
        let _decoded = decode_responses_shapes::<super::OutputText>(&bytes, limits);
    });
}

#[test]
fn decoder_rejects_missing_required_record_fields() {
    let limits = WireDecodeLimits {
        max_json_bytes: 1000,
        max_event_bytes: 1000,
        max_items: 100,
    };
    assert!(
        decode_responses_shapes::<super::OutputText>(
            br#"{"type":"output_text","text":"hello"}"#,
            limits
        )
        .is_err()
    );
}

#[rstest]
#[case::commentary("commentary")]
#[case::final_answer("final_answer")]
fn phase_literals(#[case] phase: &str) {
    let item: super::GenericResponseOutputItem = serde_json::from_value(json!({"type":"message","id":"m","status":"completed","role":"assistant","content":[],"phase":phase})).unwrap();
    assert_eq!(serde_json::to_value(item).unwrap()["phase"], phase);
    let call: super::OutputFunctionToolCall = serde_json::from_value(json!({"type":null,"id":null,"arguments":null,"call_id":null,"name":null,"status":"completed","phase":phase})).unwrap();
    assert_eq!(serde_json::to_value(call).unwrap()["phase"], phase);
}

#[rstest]
#[case::in_progress("in_progress")]
#[case::completed("completed")]
#[case::incomplete("incomplete")]
fn function_and_custom_status(#[case] status: &str) {
    let call: super::OutputFunctionToolCall = serde_json::from_value(
        json!({"type":null,"id":null,"arguments":null,"call_id":null,"name":null,"status":status}),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(call).unwrap()["phase"], json!(null));
    let custom: super::CustomToolCallOutputItem = serde_json::from_value(
        json!({"type":"custom_tool_call","call_id":"c","name":"f","input":"","status":status}),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(custom).unwrap()["status"], status);
}

#[rstest]
#[case::missing_query(r#"{}"#, r#"{}"#, "completed")]
#[case::array_failure(
    r#"null"#,
    r#"{"content":[{"type":"web_search_tool_result_error"}]}"#,
    "completed"
)]
#[case::non_mapping(r#"[]"#, r#"3"#, "completed")]
#[case::empty_query(r#"{"query":""}"#, r#"{"content":"text"}"#, "completed")]
#[case::wrong_failure(r#"{}"#, r#"{"content":{"type":"other"}}"#, "completed")]
fn search_empty_sources(#[case] input: &str, #[case] result: &str, #[case] status: &str) {
    let built = build_web_search_call(
        "t",
        &serde_json::from_str(input).unwrap(),
        &serde_json::from_str(result).unwrap(),
        None,
    );
    assert_eq!(
        serde_json::to_value(built).unwrap(),
        json!({"type":"web_search_call","id":"ws_t","status":status,"action":{"type":"search","query":"","queries":[],"sources":[]}})
    );
}

#[test]
fn interpreter_stdout_only() {
    let built = build_interpreter_logs(&serde_json::from_value(json!({"stdout":"out"})).unwrap());
    assert_eq!(
        serde_json::to_value(built).unwrap(),
        json!([{"type":"logs","logs":"out"}])
    );
}

#[rstest]
#[case::text("type")]
#[case::content("text")]
#[case::annotations("annotations")]
fn text_fields_are_required(#[case] field: &str) {
    let mut record = json!({"type":null,"text":null,"annotations":null});
    record.as_object_mut().unwrap().remove(field);
    assert!(serde_json::from_value::<super::OutputText>(record).is_err());
}

#[rstest]
#[case::kind("type")]
#[case::start("start_index")]
#[case::end("end_index")]
#[case::url("url")]
#[case::title("title")]
fn annotations_are_required_nullable(#[case] field: &str) {
    let mut record =
        json!({"type":null,"start_index":null,"end_index":null,"url":null,"title":null});
    let parsed: super::OutputAnnotation = serde_json::from_value(record.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), record);
    record.as_object_mut().unwrap().remove(field);
    assert!(serde_json::from_value::<super::OutputAnnotation>(record).is_err());
}

#[test]
fn decoded_id_preserves_absence_and_null() {
    let empty: super::DecodedResponseId = serde_json::from_str("{}").unwrap();
    assert_eq!(serde_json::to_value(empty).unwrap(), json!({}));
    let full = json!({"custom_llm_provider":null,"model_id":"m","response_id":"r"});
    let parsed: super::DecodedResponseId = serde_json::from_value(full.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), full);
    assert!(serde_json::from_str::<super::DecodedResponseId>(r#"{"response_id":null}"#).is_err());
}

#[test]
fn decoder_counts_nested_and_duplicate_members() {
    let limits = WireDecodeLimits {
        max_json_bytes: 1000,
        max_event_bytes: 1000,
        max_items: 3,
    };
    assert!(
        decode_responses_shapes::<super::DecodedResponseId>(br#"{"a":{"b":[1]}}"#, limits).is_ok()
    );
    assert!(matches!(
        decode_responses_shapes::<super::DecodedResponseId>(
            br#"{"a":1,"a":2,"a":3,"a":4}"#,
            limits
        ),
        Err(super::wire::ShapeDecodeError::TooManyItems)
    ));
    assert!(decode_responses_shapes::<super::OutputText>(b"{", limits).is_err());
}

#[test]
fn decoder_allows_exact_byte_limits_and_bounded_depth() {
    let exact = WireDecodeLimits {
        max_json_bytes: 2,
        max_event_bytes: 2,
        max_items: 0,
    };
    assert!(decode_responses_shapes::<super::DecodedResponseId>(b"{}", exact).is_ok());
    let limits = WireDecodeLimits {
        max_json_bytes: 1000,
        max_event_bytes: 1000,
        max_items: 1000,
    };
    let boundary = format!("{}null{}", "[".repeat(128), "]".repeat(128));
    assert!(decode_responses_shapes::<Box<RawValue>>(boundary.as_bytes(), limits).is_ok());
    let too_deep = format!("[{boundary}]");
    assert!(matches!(
        decode_responses_shapes::<Box<RawValue>>(too_deep.as_bytes(), limits),
        Err(super::wire::ShapeDecodeError::TooDeep)
    ));
}

#[rstest]
#[case::arguments("arguments")]
#[case::call_id("call_id")]
#[case::name("name")]
#[case::kind("type")]
#[case::id("id")]
#[case::status("status")]
fn function_fields_are_required(#[case] field: &str) {
    let mut record = json!({"arguments":null,"call_id":null,"name":null,"type":null,"id":null,"status":"completed"});
    record.as_object_mut().unwrap().remove(field);
    assert!(serde_json::from_value::<super::OutputFunctionToolCall>(record).is_err());
}

#[test]
fn private_values_are_never_deserialized_or_serialized() {
    let record: DeleteResponseResult = serde_json::from_str(
        r#"{"id":"r","object":"response","deleted":true,"_hidden_params":{"private":"value"}}"#,
    )
    .unwrap();
    assert!(record._hidden_params.is_empty());
    assert_eq!(
        serde_json::to_value(record).unwrap(),
        json!({"id":"r","object":"response","deleted":true})
    );
}

#[test]
fn populated_text_and_interpreter_outputs_round_trip() {
    let text = json!({"type":"output_text","text":"hello","annotations":[{"type":"url_citation","start_index":0,"end_index":5,"url":"https://a.test","title":"a"}]});
    let output: super::OutputText = serde_json::from_value(text.clone()).unwrap();
    assert_eq!(serde_json::to_value(output).unwrap(), text);
    let interpreter = json!({"type":"code_interpreter_call","id":"i","code":"print(1)","container_id":"c","status":"completed","outputs":[{"type":"logs","logs":"1"}]});
    let call: super::OutputCodeInterpreterCall =
        serde_json::from_value(interpreter.clone()).unwrap();
    assert_eq!(serde_json::to_value(call).unwrap(), interpreter);
}

#[rstest]
#[case::kind("type")]
#[case::id("id")]
#[case::status("status")]
#[case::role("role")]
#[case::content("content")]
fn generic_fields_are_required(#[case] field: &str) {
    let mut record =
        json!({"type":"message","id":"m","status":"completed","role":"assistant","content":[]});
    record.as_object_mut().unwrap().remove(field);
    assert!(serde_json::from_value::<super::GenericResponseOutputItem>(record).is_err());
}

#[rstest]
#[case::code("code")]
#[case::container("container_id")]
#[case::outputs("outputs")]
fn interpreter_fields_are_required_nullable(#[case] field: &str) {
    let mut record = json!({"type":"code_interpreter_call","id":"i","status":"completed","code":null,"container_id":null,"outputs":null});
    record.as_object_mut().unwrap().remove(field);
    assert!(serde_json::from_value::<super::OutputCodeInterpreterCall>(record).is_err());
}

#[test]
fn rejects_invalid_literals_and_missing_image_result() {
    assert!(
        serde_json::from_value::<super::CustomToolCallOutputItem>(
            json!({"type":"custom_tool_call","call_id":"c","name":"f","input":"","status":"failed"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<super::OutputImageGenerationCall>(
            json!({"type":"image_generation_call","id":"i","status":"completed"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<super::OutputCodeInterpreterCallLog>(
            json!({"type":"image","logs":"text"})
        )
        .is_err()
    );
}
