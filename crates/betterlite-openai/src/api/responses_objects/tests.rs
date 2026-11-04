#![cfg(test)]

use super::request_params::ResponsesApiRequestParams;
use super::{InputTokensDetails, ResponseApiUsage, ResponsesApiResponse, ResponsesApiStatus};
use crate::api::responses_shapes::wire::ShapeDecodeError;
use rstest::rstest;
use serde_json::{Value, error::Category};
use std::fs;

#[rstest]
#[case::minimal(include_str!("tests/fixtures/sdk-input.json"), include_str!("tests/fixtures/sdk-expected.json"), include_str!("tests/fixtures/sdk-text.txt"))]
#[case::nested(include_str!("tests/fixtures/sdk-rich-input.json"), include_str!("tests/fixtures/sdk-rich-expected.json"), include_str!("tests/fixtures/sdk-rich-text.txt"))]
#[case::fallback(include_str!("tests/fixtures/sdk-fallback-input.json"), include_str!("tests/fixtures/sdk-fallback-expected.json"), include_str!("tests/fixtures/sdk-fallback-text.txt"))]
fn sdk_output_defaults(#[case] input: &str, #[case] expected_json: &str, #[case] text: &str) {
    let expected = parse_json(expected_json);
    let response: ResponsesApiResponse = serde_json::from_str(input).unwrap();
    assert_eq!(serde_json::to_value(&response).unwrap(), expected);
    assert_eq!(response.output_text(), text);
}

#[test]
fn response_defaults_and_output_text() {
    let response = serde_json::from_value::<ResponsesApiResponse>(parse_json(
        r#"{"id":"resp_a","created_at":1,"output":[]}"#,
    ))
    .unwrap();
    assert_eq!(response.status, None);
    assert_eq!(response.output_text(), "");
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["status"], parse_json("null"));
    assert!(value.get("_hidden_params").is_none());
}

#[test]
fn usage_cost_mapping_is_normalized() {
    let usage = serde_json::from_value::<ResponseApiUsage>(parse_json(
        r#"{"input_tokens":3,"output_tokens":2,"total_tokens":5,"cost":{"total_cost":0.125,"provider":"ignored"}}"#,
    ))
    .unwrap();
    assert_eq!(
        usage.cost.map(|cost| cost.to_string()),
        Some("0.125".to_owned())
    );
}

#[rstest]
#[case::number("0.25", Some("0.25"))]
#[case::empty_mapping("{}", None)]
#[case::scientific("1e-4", Some("0.0001"))]
#[case::null_mapping(r#"{"total_cost":null}"#, None)]
fn cost_accepts_numbers_and_empty_mapping(
    #[case] encoded_cost: &str,
    #[case] expected: Option<&str>,
) {
    let encoded_usage =
        format!(r#"{{"input_tokens":3,"output_tokens":2,"total_tokens":5,"cost":{encoded_cost}}}"#);
    let usage = serde_json::from_value::<ResponseApiUsage>(parse_json(&encoded_usage)).unwrap();
    assert_eq!(
        usage.cost.map(|cost| cost.to_string()),
        expected.map(str::to_owned)
    );
}

#[rstest]
#[case::text(r#""0.25""#)]
#[case::list("[]")]
#[case::boolean("true")]
#[case::bad_mapping(r#"{"total_cost":"bad"}"#)]
fn usage_cost_rejects_wrong_types(#[case] cost: &str) {
    let encoded =
        format!(r#"{{"input_tokens":3,"output_tokens":2,"total_tokens":5,"cost":{cost}}}"#);
    let error = serde_json::from_str::<ResponseApiUsage>(&encoded).unwrap_err();
    assert_eq!(error.classify(), Category::Data);
}

#[test]
fn output_serialization_filters_only_reasoning_null_fields() {
    let response = serde_json::from_value::<ResponsesApiResponse>(parse_json(
        r#"{"id":"resp_a","created_at":1,"output":[{"type":"reasoning","id":"rs_a","status":null,"content":null,"encrypted_content":null,"summary":null,"provider":1},{"type":"message","id":"m_a","status":null,"content":[],"role":"assistant","summary":null}]}"#,
    ))
    .unwrap();
    let output = serde_json::to_value(response).unwrap()["output"].clone();
    assert!(output[0].get("status").is_none());
    assert!(output[0].get("content").is_none());
    assert!(output[0].get("encrypted_content").is_none());
    assert_eq!(output[0].get("summary"), Some(&Value::Null));
    assert_eq!(output[0]["provider"], parse_json("1"));
    assert_eq!(output[1].get("status"), Some(&Value::Null));
}

#[test]
fn output_text_joins_only_message_output_text_blocks() {
    let response = serde_json::from_value::<ResponsesApiResponse>(parse_json(
        r#"{"id":"resp_a","created_at":1,"output":[{"type":"message","id":"m_a","status":"completed","role":"assistant","content":[{"type":"output_text","text":"a","annotations":[]},{"type":"output_text","text":null,"annotations":[]},{"type":"output_text","text":"b","annotations":[]}]},{"type":"refusal","refusal":"no"},{"type":"message","content":[{"type":"output_text","text":"x"}]}]}"#,
    ))
    .unwrap();
    assert_eq!(response.output_text(), "abx");
}

#[rstest]
#[case::missing_id(r#"{"created_at":1,"output":[]}"#)]
#[case::missing_created_at(r#"{"id":"resp_a","output":[]}"#)]
#[case::missing_output(r#"{"id":"resp_a","created_at":1}"#)]
fn response_requires_id_created_at_and_output(#[case] encoded: &str) {
    let error = serde_json::from_value::<ResponsesApiResponse>(parse_json(encoded)).unwrap_err();
    assert_eq!(error.classify(), Category::Data);
}

#[rstest]
#[case::none(r#"{"effort":"none","detail":1}"#)]
#[case::xhigh(r#"{"effort":"xhigh","detail":1}"#)]
fn status_is_open_and_reasoning_effort_is_preserved(#[case] reasoning: &str) {
    let response = serde_json::from_value::<ResponsesApiResponse>(parse_json(&format!(
        r#"{{"id":"resp_a","created_at":1,"output":[],"status":"future_status","truncation":"disabled","reasoning":{reasoning}}}"#
    )))
    .unwrap();
    assert_eq!(response.status.as_deref(), Some("future_status"));
    assert_eq!(
        serde_json::to_value(&response.reasoning).unwrap(),
        parse_json(reasoning)
    );
    let error = serde_json::from_value::<ResponsesApiResponse>(parse_json(
        r#"{"id":"resp_a","created_at":1,"output":[],"truncation":"other"}"#,
    ))
    .unwrap_err();
    assert_eq!(error.classify(), Category::Data);
}

#[test]
fn request_fields_keep_omission_distinct_from_null() {
    let request = serde_json::from_value::<ResponsesApiRequestParams>(parse_json(
        r#"{"input":"hello","model":"gpt-test","instructions":null,"partial_images":0}"#,
    ))
    .unwrap();
    assert_eq!(request.instructions, Some(None));
    assert_eq!(request.include, None);
    assert_eq!(request.partial_images, Some(Some(0)));
}

#[rstest]
#[case::input(r#"{"model":"gpt-test"}"#)]
#[case::model(r#"{"input":"hello"}"#)]
fn request_required_fields(#[case] input: &str) {
    let error = serde_json::from_str::<ResponsesApiRequestParams>(input).unwrap_err();
    assert_eq!(error.classify(), Category::Data);
}

#[rstest]
#[case::completed("completed", ResponsesApiStatus::Completed)]
#[case::failed("failed", ResponsesApiStatus::Failed)]
#[case::in_progress("in_progress", ResponsesApiStatus::InProgress)]
#[case::cancelled("cancelled", ResponsesApiStatus::Cancelled)]
#[case::queued("queued", ResponsesApiStatus::Queued)]
#[case::incomplete("incomplete", ResponsesApiStatus::Incomplete)]
fn status_declaration(#[case] value: &str, #[case] status: ResponsesApiStatus) {
    assert_eq!(
        serde_json::to_value(status).unwrap(),
        Value::String(value.to_owned())
    );
}

#[test]
fn request_declarations_accept_named_optional_fields_without_bounds_checks() {
    let input = parse_json(
        r#"{"input":"hello","model":"gpt-test","include":null,"instructions":null,"max_output_tokens":-1,"metadata":null,"parallel_tool_calls":null,"previous_response_id":null,"reasoning":null,"store":null,"background":null,"stream":null,"temperature":0.25,"text":null,"tool_choice":null,"tools":null,"top_p":0.75,"truncation":"unexpected","user":null,"service_tier":null,"safety_identifier":null,"prompt":null,"max_tool_calls":null,"prompt_cache_key":null,"prompt_cache_retention":null,"prompt_cache_options":null,"stream_options":{"include_obfuscation":null},"top_logprobs":null,"partial_images":0,"context_management":[{"compact_threshold":0}]}"#,
    );
    let request: ResponsesApiRequestParams = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(request.max_output_tokens, Some(Some(-1)));
    assert_eq!(request.partial_images, Some(Some(0)));
    assert_eq!(
        request.stream_options,
        Some(Some(super::request_params::ResponsesApiStreamOptions {
            include_obfuscation: Some(None)
        }))
    );
    assert_eq!(serde_json::to_value(request).unwrap(), input);
}

#[test]
fn usage_defaults_and_preserves_extra_fields() {
    let usage = serde_json::from_value::<ResponseApiUsage>(parse_json(
        r#"{"input_tokens":3,"output_tokens":2,"total_tokens":5,"input_tokens_details":{"provider_count":7}}"#,
    ))
    .unwrap();
    assert_eq!(
        usage
            .input_tokens_details
            .as_ref()
            .map(|details| details.cached_tokens),
        Some(0)
    );
    assert_eq!(
        usage
            .input_tokens_details
            .as_ref()
            .map(|details| serde_json::to_value(&details.extra["provider_count"]).unwrap()),
        Some(parse_json("7"))
    );
    assert_eq!(usage.cost, None);
    let error = serde_json::from_value::<ResponseApiUsage>(parse_json(
        r#"{"input_tokens":3,"output_tokens":2}"#,
    ))
    .unwrap_err();
    assert_eq!(error.classify(), Category::Data);
}

#[test]
fn input_details_null_defaults_are_serialized() {
    let details = serde_json::from_value::<InputTokensDetails>(parse_json("{}")).unwrap();
    let value = serde_json::to_value(details).unwrap();
    assert_eq!(value["cached_tokens"], parse_json("0"));
    assert_eq!(value["audio_tokens"], parse_json("null"));
    assert_eq!(value["video_tokens"], parse_json("null"));
}

#[test]
fn output_details_null_defaults_are_serialized() {
    let details = serde_json::from_value::<super::OutputTokensDetails>(parse_json("{}")).unwrap();
    let value = serde_json::to_value(details).unwrap();
    assert_eq!(value["audio_tokens"], parse_json("null"));
    assert_eq!(value["reasoning_tokens"], parse_json("null"));
    assert_eq!(value["text_tokens"], parse_json("null"));
}

#[rstest]
#[case::missing_null(r#"{"web_search":null}"#)]
#[case::zero(r#"{"web_search":{"num_requests":0}}"#)]
#[case::positive(r#"{"web_search":{"num_requests":2}}"#)]
fn accepts_valid_tool_usage_counts(#[case] encoded: &str) {
    serde_json::from_value::<super::ResponsesToolUsage>(parse_json(encoded)).unwrap();
}

#[test]
fn rejects_negative_tool_usage_count() {
    let error = serde_json::from_value::<super::ResponsesToolUsage>(parse_json(
        r#"{"web_search":{"num_requests":-1}}"#,
    ))
    .unwrap_err();
    assert_eq!(error.classify(), Category::Syntax);
}

#[test]
fn replays_corpus() {
    let limits = super::wire::WireDecodeLimits {
        max_json_bytes: 1_000_000,
        max_event_bytes: 1_000_000,
        max_items: 10_000,
    };
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/responses_objects");
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .for_each(|path| {
            let bytes = fs::read(path).unwrap();
            let _response = super::decode_responses_objects::<ResponsesApiResponse>(&bytes, limits);
        });
}

fn parse_json(encoded: &str) -> Value {
    serde_json::from_str(encoded).unwrap()
}

#[test]
fn usage_cost() {
    let usage: ResponseApiUsage = serde_json::from_str(r#"{"input_tokens":3,"output_tokens":2,"total_tokens":5,"input_tokens_details":{"cached_tokens":null,"cached_tokens_details":{}},"output_tokens_details":{"provider_count":7},"provider_count":7}"#).unwrap();
    let value = serde_json::to_value(usage).unwrap();
    assert_eq!(
        value["input_tokens_details"]["cached_tokens"],
        parse_json("0")
    );
    assert_eq!(
        value["input_tokens_details"]["cached_tokens_details"],
        parse_json(r#"{"audio_tokens":null,"text_tokens":null,"image_tokens":null}"#)
    );
    assert_eq!(
        value["output_tokens_details"]["provider_count"],
        parse_json("7")
    );
    assert_eq!(value["provider_count"], parse_json("7"));
}

#[test]
fn response_defaults() {
    let response: ResponsesApiResponse = serde_json::from_str(
        r#"{"id":"r","created_at":1,"output":[],"incomplete_details":{},"provider":7}"#,
    )
    .unwrap();
    let value = serde_json::to_value(response).unwrap();
    assert!(value.get("_hidden_params").is_none());
    assert_eq!(
        value["incomplete_details"],
        parse_json(r#"{"reason":null}"#)
    );
    assert_eq!(value["provider"], parse_json("7"));
}

#[test]
fn hidden_params_input_is_not_public() {
    let response: ResponsesApiResponse = serde_json::from_str(
        r#"{"id":"r","created_at":1,"output":[],"_hidden_params":{"probe":7},"provider":1}"#,
    )
    .unwrap();
    let value = serde_json::to_value(response).unwrap();
    assert!(value.get("_hidden_params").is_none());
    assert_eq!(value["provider"], parse_json("1"));
}

#[test]
fn typed_non_message_does_not_contribute_text() {
    let mut response: ResponsesApiResponse =
        serde_json::from_str(r#"{"id":"r","created_at":1,"output":[]}"#).unwrap();
    let item = serde_json::from_str(r#"{"type":"other","id":"m","role":"assistant","status":"completed","content":[{"type":"output_text","text":"ignore","annotations":[]}]}"#).unwrap();
    response
        .output
        .push(super::ResponseOutputItem::Message(item));
    assert_eq!(response.output_text(), "");
}

#[rstest]
#[case::message(true)]
#[case::function(false)]
fn typed_reasoning_filters_nulls(#[case] message: bool) {
    let mut response: ResponsesApiResponse =
        serde_json::from_str(r#"{"id":"r","created_at":1,"output":[]}"#).unwrap();
    let encoded = r#"{"type":"reasoning","id":"rs","role":"assistant","status":"completed","content":[],"arguments":"","call_id":"c","name":"f","encrypted_content":null,"summary":null}"#;
    let item = if message {
        super::ResponseOutputItem::Message(serde_json::from_str(encoded).unwrap())
    } else {
        super::ResponseOutputItem::FunctionCall(serde_json::from_str(encoded).unwrap())
    };
    response.output.push(item);
    let value = serde_json::to_value(response).unwrap();
    assert!(value["output"][0].get("encrypted_content").is_none());
    assert_eq!(value["output"][0].get("summary"), Some(&Value::Null));
}

#[test]
fn reasoning_nulls() {
    let response: ResponsesApiResponse = serde_json::from_str(r#"{"id":"r","created_at":1,"output":[{"id":"reason","type":"reasoning","summary":[],"status":"completed","encrypted_content":"encrypted","content":[{"type":"reasoning_text","text":"why"}]}]}"#).unwrap();
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["output"][0]["status"], "completed");
    assert_eq!(value["output"][0]["encrypted_content"], "encrypted");
    assert_eq!(value["output"][0]["content"][0]["text"], "why");
}

#[test]
fn output_text_join() {
    let mut response: ResponsesApiResponse = serde_json::from_str(r#"{"id":"r","created_at":1,"output":[{"type":"message","content":[{"type":"output_text","text":"a"},{"type":"output_text","text":null},{"type":"output_text"},{"type":"refusal","text":"ignore"}]},{"type":"other","content":[{"type":"output_text","text":"ignore"}]},{"type":"message"}]}"#).unwrap();
    let typed = serde_json::from_str(r#"{"type":"message","id":"m","role":"assistant","status":"completed","content":[{"type":"output_text","text":"b","annotations":[]}] }"#).unwrap();
    response
        .output
        .push(super::ResponseOutputItem::Message(typed));
    assert_eq!(response.output_text(), "ab");
}

#[test]
fn tool_usage_count() {
    let usage: super::ResponsesToolUsage = serde_json::from_str("{}").unwrap();
    assert_eq!(
        serde_json::to_value(usage).unwrap(),
        parse_json(r#"{"web_search":null}"#)
    );
}

#[test]
fn sdk_output_variants_are_typed() {
    let response: ResponsesApiResponse =
        serde_json::from_str(include_str!("tests/fixtures/sdk-input.json")).unwrap();
    assert_eq!(response.output.len(), 28);
    assert!(
        response
            .output
            .iter()
            .all(|item| matches!(item, super::ResponseOutputItem::Sdk(_)))
    );
}

#[test]
fn response_fields() {
    let responses: Vec<ResponsesApiResponse> =
        serde_json::from_str(include_str!("tests/fixtures/fields-input.json")).unwrap();
    assert_eq!(
        serde_json::to_value(responses).unwrap(),
        parse_json(include_str!("tests/fixtures/fields-expected.json"))
    );
}

#[rstest]
#[case::choice_number(r#""tool_choice":3"#)]
#[case::choice_unknown(r#""tool_choice":"future""#)]
#[case::choice_missing_name(r#""tool_choice":{"type":"function"}"#)]
#[case::text_number(r#""text":3"#)]
fn response_field_types(#[case] field: &str) {
    let input = format!(r#"{{"id":"r","created_at":1,"output":[],{field}}}"#);
    let error = serde_json::from_str::<ResponsesApiResponse>(&input).unwrap_err();
    assert_eq!(error.classify(), Category::Data);
}

#[rstest]
#[case::body_limit(1, 100, 100)]
#[case::event_limit(100, 1, 100)]
#[case::item_limit(100, 100, 1)]
fn decoder_enforces_limits(
    #[case] max_json_bytes: usize,
    #[case] max_event_bytes: usize,
    #[case] max_items: usize,
) {
    let limits = super::wire::WireDecodeLimits {
        max_json_bytes,
        max_event_bytes,
        max_items,
    };
    let error = super::decode_responses_objects::<ResponsesApiResponse>(
        br#"{"id":"r","created_at":1,"output":[]}"#,
        limits,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        super::wire::ResponsesObjectsDecodeError::ResponsesShapes(
            ShapeDecodeError::JsonTooLarge | ShapeDecodeError::TooManyItems
        )
    ));
}

#[test]
fn decoder_handles_valid_and_malformed_json() {
    let limits = super::wire::WireDecodeLimits {
        max_json_bytes: 100,
        max_event_bytes: 100,
        max_items: 100,
    };
    let response = super::decode_responses_objects::<ResponsesApiResponse>(
        br#"{"id":"r","created_at":1,"output":[]}"#,
        limits,
    )
    .unwrap();
    assert_eq!(response.id, "r");
    let error = super::decode_responses_objects::<ResponsesApiResponse>(b"{", limits).unwrap_err();
    assert!(matches!(
        error,
        super::wire::ResponsesObjectsDecodeError::ResponsesShapes(ShapeDecodeError::Json(_))
    ));
}
