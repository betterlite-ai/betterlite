#![cfg(test)]

use super::request_params::ResponsesApiRequestParams;
use super::{InputTokensDetails, ResponseApiUsage, ResponsesApiResponse};
use rstest::rstest;
use serde_json::{Value, error::Category};
use std::fs;

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
    assert_eq!(output[0]["summary"], parse_json("null"));
    assert_eq!(output[0]["provider"], parse_json("1"));
    assert_eq!(output[1]["status"], parse_json("null"));
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

#[test]
fn request_declarations_accept_named_optional_fields_without_bounds_checks() {
    let request = serde_json::from_value::<ResponsesApiRequestParams>(parse_json(
        r#"{"input":"hello","model":"gpt-test","include":null,"instructions":null,"max_output_tokens":-1,"metadata":null,"parallel_tool_calls":null,"previous_response_id":null,"reasoning":null,"store":null,"background":null,"stream":null,"temperature":null,"text":null,"tool_choice":null,"tools":null,"top_p":null,"truncation":"unexpected","user":null,"service_tier":null,"safety_identifier":null,"prompt":null,"max_tool_calls":null,"prompt_cache_key":null,"prompt_cache_retention":null,"prompt_cache_options":null,"stream_options":{"include_obfuscation":null},"top_logprobs":null,"partial_images":0,"context_management":[{"compact_threshold":0}]}"#,
    ))
    .unwrap();
    assert_eq!(request.max_output_tokens, Some(Some(-1)));
    assert_eq!(request.partial_images, Some(Some(0)));
    assert_eq!(
        request.stream_options,
        Some(Some(super::request_params::ResponsesApiStreamOptions {
            include_obfuscation: Some(None)
        }))
    );
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
