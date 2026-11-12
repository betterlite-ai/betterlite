#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use uuid::Uuid;

use crate::gemini_candidates::choice::Choices;
use crate::gemini_response::complete::{
    CompleteRequest, CompleteResponse, RawResponse, SERVICE_TIER_HEADER,
    assemble_complete_response, served_model_name, transform_response,
};
use crate::gemini_response::decode::{
    GenerateContentResponse, MAX_RESPONSE_BYTES, decode_generate_content,
};
use crate::gemini_response::error::GenerateContentError;
use crate::gemini_response_parts::projection::CallMode;

const GENERATED: &str = "0123456789abcdef0123456789abcdef";
const GENERATED_ID: &str = "chatcmpl-01234567-89ab-cdef-0123-456789abcdef";
const REQUESTED_MODEL: &str = "gemini-2.5-flash";
const USAGE: &str =
    r#""usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5}"#;

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn request(model: &str) -> CompleteRequest<'_> {
    CompleteRequest {
        model,
        now: 1_700_000_000,
        call_mode: CallMode::Tools,
    }
}

fn fixed_id() -> Uuid {
    Uuid::parse_str(GENERATED).unwrap()
}

fn decode(source: &str) -> GenerateContentResponse {
    decode_generate_content(source.as_bytes()).unwrap()
}

fn expected(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn complete_with(source: &str, header_pairs: &[(&str, &str)]) -> CompleteResponse {
    let body = decode_generate_content(source.as_bytes()).unwrap();
    let map = headers(header_pairs);
    assemble_complete_response(&body, &map, &request(REQUESTED_MODEL), &mut fixed_id).unwrap()
}

fn complete(source: &str) -> CompleteResponse {
    complete_with(source, &[])
}

fn choices_of(response: &CompleteResponse) -> Json {
    expected(&serde_json::to_string(&response.choices).unwrap())
}

fn finish_reasons(response: &CompleteResponse) -> Vec<String> {
    let Json::List(choices) = choices_of(response) else {
        panic!("choices are a list");
    };
    choices
        .iter()
        .map(|choice| {
            let Json::Object(fields) = choice else {
                panic!("a choice is an object");
            };
            let Some(Json::Text(reason)) = fields.get("finish_reason") else {
                panic!("a choice has a finish reason");
            };
            reason.clone()
        })
        .collect()
}

fn text_reply(extra: &str) -> String {
    format!(
        r#"{{"candidates":[{{"content":{{"parts":[{{"text":"hello"}}]}},"finishReason":"STOP"}}],{USAGE}{extra}}}"#
    )
}

#[test]
fn decode_keeps_the_typed_envelope_fields() {
    let body = decode(
        r#"{"candidates":[{"index":0,"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP"}],
            "promptFeedback":{"blockReason":"SAFETY"},
            "usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5,"trafficType":"ON_DEMAND"},
            "responseId":"resp-1","modelVersion":"gemini-2.5-pro@2025-06-17"}"#,
    );
    assert_eq!(body.candidates.unwrap().len(), 1);
    assert_eq!(
        body.prompt_feedback.unwrap().block_reason.as_deref(),
        Some("SAFETY")
    );
    assert_eq!(body.usage_metadata.unwrap().prompt_token_count, 3);
    assert_eq!(body.traffic_type.as_deref(), Some("ON_DEMAND"));
    assert_eq!(body.response_id.as_deref(), Some("resp-1"));
    assert_eq!(
        body.model_version,
        Some(Json::Text("gemini-2.5-pro@2025-06-17".to_owned()))
    );
}

#[test]
fn decode_accepts_an_empty_object() {
    let body = decode("{}");
    assert!(body.candidates.is_none());
    assert!(body.prompt_feedback.is_none());
    assert!(body.usage_metadata.is_none());
    assert!(body.traffic_type.is_none());
    assert!(body.response_id.is_none());
    assert!(body.model_version.is_none());
}

#[test]
fn decode_keeps_a_nonstring_model_version_for_the_requested_fallback() {
    let body = decode(r#"{"modelVersion":7}"#);
    assert_eq!(
        body.model_version,
        Some(Json::Number(serde_json::Number::from(7_i32)))
    );
}

#[test]
fn decode_keeps_an_empty_usage_object_as_defaulted_metadata() {
    let body = decode(r#"{"usageMetadata":{}}"#);
    let usage = body.usage_metadata.unwrap();
    assert_eq!(usage.prompt_token_count, 0);
    assert_eq!(usage.total_token_count, 0);
    assert!(body.traffic_type.is_none());
}

#[rstest]
#[case::list("[]")]
#[case::positional_list(r#"[null,null,{"totalTokenCount":1},null,null]"#)]
#[case::text(r#""text""#)]
#[case::number("7")]
#[case::null("null")]
fn decode_rejects_a_non_object_root(#[case] source: &str) {
    assert!(matches!(
        decode_generate_content(source.as_bytes()),
        Err(GenerateContentError::Json { .. })
    ));
}

#[rstest]
#[case::list(r#"{"promptFeedback":["SAFETY"],"usageMetadata":{}}"#)]
#[case::text(r#"{"promptFeedback":"SAFETY","usageMetadata":{}}"#)]
fn decode_rejects_a_non_object_prompt_feedback(#[case] source: &str) {
    let failure = decode_generate_content(source.as_bytes()).unwrap_err();
    assert!(matches!(failure, GenerateContentError::Json { .. }));
    assert!(failure.to_string().contains("promptFeedback"));
}

#[test]
fn decode_rejects_truncated_json() {
    assert!(matches!(
        decode_generate_content(br#"{"candidates": [{"content": {"parts": [{"text": "cut"#),
        Err(GenerateContentError::Json { .. })
    ));
}

#[test]
fn decode_rejects_bytes_that_are_not_utf8() {
    assert!(matches!(
        decode_generate_content(&[0x7b, 0x22, 0xff, 0x22, 0x3a, 0x31, 0x7d]),
        Err(GenerateContentError::Json { .. })
    ));
}

#[test]
fn decode_rejects_deeply_nested_arguments() {
    let depth = 200;
    let mut source =
        String::from(r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"f","args":"#);
    source.push_str(&"[".repeat(depth));
    source.push_str(&"]".repeat(depth));
    source.push_str(r#"}}]},"finishReason":"STOP"}],"usageMetadata":{}}"#);
    assert!(matches!(
        decode_generate_content(source.as_bytes()),
        Err(GenerateContentError::Json { .. })
    ));
}

#[rstest]
#[case::usage_list(r#"{"usageMetadata":[]}"#)]
#[case::string_token_count(r#"{"usageMetadata":{"promptTokenCount":"many"}}"#)]
#[case::nonstring_traffic_type(r#"{"usageMetadata":{"totalTokenCount":1,"trafficType":7}}"#)]
fn decode_rejects_invalid_usage_shapes(#[case] source: &str) {
    assert!(matches!(
        decode_generate_content(source.as_bytes()),
        Err(GenerateContentError::Json { .. })
    ));
}

#[test]
fn decode_names_the_field_that_breaks_the_typed_parse() {
    let failure = decode_generate_content(br#"{"responseId":7}"#).unwrap_err();
    assert!(failure.to_string().contains("responseId"));
}

#[test]
fn decode_rejects_malformed_candidate_content() {
    assert!(matches!(
        decode_generate_content(br#"{"candidates":[{"content":{"parts":"no"}}]}"#),
        Err(GenerateContentError::Json { .. })
    ));
}

#[rstest]
#[case::number(r#"{"candidates":[7]}"#)]
#[case::list(r#"{"candidates":[[]]}"#)]
fn decode_rejects_a_non_object_candidate(#[case] source: &str) {
    assert!(matches!(
        decode_generate_content(source.as_bytes()),
        Err(GenerateContentError::Json { .. })
    ));
}

#[test]
fn decode_rejects_a_trailing_document() {
    assert!(matches!(
        decode_generate_content(br#"{"responseId":"a"} {}"#),
        Err(GenerateContentError::Json { .. })
    ));
}

#[test]
fn decode_enforces_the_byte_limit_on_both_sides() {
    let prefix = r#"{"candidates":[],"usageMetadata":{},"pad":""#;
    let suffix = "\"}";
    let padding = MAX_RESPONSE_BYTES - prefix.len() - suffix.len();
    let mut exact = String::with_capacity(MAX_RESPONSE_BYTES);
    exact.push_str(prefix);
    exact.push_str(&"x".repeat(padding));
    exact.push_str(suffix);
    assert_eq!(exact.len(), MAX_RESPONSE_BYTES);
    drop(decode_generate_content(exact.as_bytes()).unwrap());
    drop(exact);
    let oversized = format!(r#"{{"pad":"{}"}}"#, "x".repeat(MAX_RESPONSE_BYTES));
    assert!(matches!(
        decode_generate_content(oversized.as_bytes()),
        Err(GenerateContentError::TooLarge { .. })
    ));
}

#[test]
fn decode_drops_an_empty_traffic_type() {
    let body = decode(r#"{"usageMetadata":{"totalTokenCount":1,"trafficType":""}}"#);
    assert!(body.traffic_type.is_none());
}

#[rstest]
#[case::served_prefix("gemini-2.5-pro@2025-06-17", Some("gemini-2.5-pro"))]
#[case::no_at("served-model", Some("served-model"))]
#[case::empty_prefix("@2025-06-17", Some(""))]
#[case::first_at_wins("a@b@c", Some("a"))]
#[case::empty("", None)]
fn served_model_name_splits_a_string(#[case] version: &str, #[case] served: Option<&str>) {
    assert_eq!(
        served_model_name(Some(&Json::Text(version.to_owned()))),
        served
    );
}

#[rstest]
#[case::number(Json::Number(serde_json::Number::from(7_i32)))]
#[case::null(Json::Null)]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::default()))]
#[case::boolean(Json::Bool(true))]
fn served_model_name_ignores_a_nonstring_value(#[case] version: Json) {
    assert_eq!(served_model_name(Some(&version)), None);
}

#[test]
fn served_model_name_ignores_an_absent_version() {
    assert_eq!(served_model_name(None), None);
}

#[rstest]
#[case::served("gemini-2.5-pro@2025-06-17", "gemini-2.5-pro")]
#[case::nonstring("7", REQUESTED_MODEL)]
#[case::empty(r#""""#, REQUESTED_MODEL)]
fn complete_response_uses_the_served_or_requested_model(
    #[case] version: &str,
    #[case] model: &str,
) {
    let response = complete(&text_reply(&format!(r#","modelVersion":{version}"#)));
    assert_eq!(response.model, model);
}

#[test]
fn complete_response_uses_the_generated_id_by_default() {
    let response = complete(&text_reply(""));
    assert_eq!(response.id, GENERATED_ID);
}

#[rstest]
#[case::empty(r#","responseId":"""#)]
#[case::null(r#","responseId":null"#)]
#[case::absent("")]
fn complete_response_keeps_the_generated_id_for_a_falsy_response_id(#[case] extra: &str) {
    let response = complete(&text_reply(extra));
    assert_eq!(response.id, GENERATED_ID);
}

#[test]
fn complete_response_replaces_the_id_with_a_truthy_response_id() {
    let response = complete(&text_reply(r#","responseId":"resp-1""#));
    assert_eq!(response.id, "resp-1");
}

#[test]
fn complete_response_projects_the_candidate_text() {
    let response = complete(&text_reply(""));
    assert_eq!(
        choices_of(&response),
        expected(
            r#"[{
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "hello",
                    "reasoning_content": null,
                    "thinking_blocks": [],
                    "tool_calls": null,
                    "function_call": null,
                    "images": [],
                    "audio": null,
                    "annotations": null,
                    "provider_specific_fields": null
                },
                "logprobs": null,
                "native_finish_reason": "STOP"
            }]"#
        )
    );
}

#[rstest]
#[case::empty_list(r#"{"candidates":[],"usageMetadata":{"totalTokenCount":4}}"#)]
#[case::absent(r#"{"usageMetadata":{"totalTokenCount":4}}"#)]
#[case::null(r#"{"candidates":null,"usageMetadata":{"totalTokenCount":4}}"#)]
fn an_empty_candidate_list_leaves_no_choices_and_empty_metadata(#[case] source: &str) {
    let response = complete(source);
    assert_eq!(choices_of(&response), expected("[]"));
    let metadata = response.metadata.unwrap();
    assert!(metadata.grounding_metadata.is_empty());
    assert!(metadata.url_context_metadata.is_empty());
    assert!(metadata.safety_results.is_empty());
    assert!(metadata.citation_metadata.is_empty());
    assert_eq!(response.usage.total_tokens, 4);
}

#[test]
fn complete_metadata_carries_the_candidate_lists() {
    let response = complete(
        r#"{"candidates":[{"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP",
            "groundingMetadata":{"webSearchQueries":["q"]},
            "safetyRatings":[{"category":"HARM_CATEGORY_HATE_SPEECH"}],
            "citationMetadata":{"citations":[{"uri":"https://a"}]},
            "urlContextMetadata":{"urlMetadata":[{"retrievedUrl":"https://b"}]}}],
            "usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}"#,
    );
    let metadata = response.metadata.unwrap();
    assert_eq!(
        metadata.grounding_metadata,
        vec![expected(r#"{"webSearchQueries": ["q"]}"#)]
    );
    assert_eq!(
        metadata.safety_results,
        vec![expected(r#"[{"category": "HARM_CATEGORY_HATE_SPEECH"}]"#)]
    );
    assert_eq!(
        metadata.citation_metadata,
        vec![expected(r#"{"citations": [{"uri": "https://a"}]}"#)]
    );
    assert_eq!(
        metadata.url_context_metadata,
        vec![expected(
            r#"{"urlMetadata": [{"retrievedUrl": "https://b"}]}"#
        )]
    );
}

#[test]
fn complete_response_retains_traffic_type_under_provider_fields() {
    let response = complete(
        r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5,
                "trafficType":"ON_DEMAND"}}"#,
    );
    assert_eq!(
        response
            .provider_specific_fields
            .map(|fields| fields.traffic_type),
        Some("ON_DEMAND".to_owned())
    );
}

#[test]
fn an_ordinary_reply_has_no_traffic_type() {
    let response = complete(&text_reply(""));
    assert!(response.provider_specific_fields.is_none());
}

#[rstest]
#[case::standard("x-gemini-service-tier", "STANDARD", Some("default"))]
#[case::mixed_case_header("X-Gemini-Service-Tier", "Standard", Some("default"))]
#[case::priority("x-gemini-service-tier", "Priority", Some("priority"))]
#[case::flex("x-gemini-service-tier", "flex", Some("flex"))]
#[case::empty("x-gemini-service-tier", "", None)]
fn complete_response_maps_the_service_tier_header(
    #[case] name: &str,
    #[case] value: &str,
    #[case] tier: Option<&str>,
) {
    let response = complete_with(&text_reply(""), &[(name, value)]);
    assert_eq!(response.service_tier.as_deref(), tier);
}

#[test]
fn repeated_service_tier_headers_join_with_a_comma() {
    let response = complete_with(
        &text_reply(""),
        &[
            (SERVICE_TIER_HEADER, "priority"),
            (SERVICE_TIER_HEADER, "flex"),
        ],
    );
    assert_eq!(response.service_tier.as_deref(), Some("priority, flex"));
}

#[test]
fn a_prompt_block_bypasses_the_rest_of_the_enrichment() {
    let response = complete_with(
        r#"{"responseId":"ignored","modelVersion":"served@rev",
            "promptFeedback":{"blockReason":"SAFETY"},
            "candidates":[{"content":{"parts":[{"text":"ignored"}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":7,"totalTokenCount":7,"trafficType":"ON_DEMAND"}}"#,
        &[(SERVICE_TIER_HEADER, "priority")],
    );
    assert_eq!(response.id, GENERATED_ID);
    assert_eq!(response.model, "served");
    assert_eq!(
        choices_of(&response),
        expected(
            r#"[{
                "index": 0,
                "finish_reason": "content_filter",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": null,
                    "thinking_blocks": null,
                    "tool_calls": null,
                    "function_call": null,
                    "images": null,
                    "audio": null,
                    "annotations": null,
                    "provider_specific_fields": null
                },
                "logprobs": null,
                "native_finish_reason": null
            }]"#
        )
    );
    assert!(response.metadata.is_none());
    assert!(response.provider_specific_fields.is_none());
    assert!(response.service_tier.is_none());
    assert_eq!(response.usage.prompt_tokens, 7);
    assert!(response.usage.prompt_tokens_details.is_none());
}

#[test]
fn a_flagged_first_candidate_blocks_the_reply() {
    let response = complete(
        r#"{"candidates":[{"finishReason":"SAFETY"}],"usageMetadata":{"promptTokenCount":2,"totalTokenCount":2}}"#,
    );
    assert!(response.metadata.is_none());
    assert_eq!(finish_reasons(&response), vec!["content_filter"]);
}

#[test]
fn a_flagged_later_candidate_stays_an_ordinary_choice() {
    let response = complete(
        r#"{"candidates":[
            {"content":{"parts":[{"text":"first"}]},"finishReason":"STOP"},
            {"content":{"parts":[{"text":"second"}]},"finishReason":"SAFETY"}],
            "usageMetadata":{"promptTokenCount":2,"totalTokenCount":2}}"#,
    );
    assert!(response.metadata.is_some());
    assert_eq!(
        finish_reasons(&response),
        vec!["stop".to_owned(), "content_filter".to_owned()]
    );
}

#[test]
fn a_blocked_reply_without_usage_metadata_is_a_422_response() {
    let map = headers(&[(SERVICE_TIER_HEADER, "priority")]);
    let raw = RawResponse {
        headers: &map,
        body: br#"{"promptFeedback":{"blockReason":"SAFETY"}}"#,
    };
    let failure = transform_response(&raw, &request(REQUESTED_MODEL), &mut fixed_id).unwrap_err();
    assert_eq!(failure.status_code, 422);
}

#[test]
fn a_complete_reply_without_usage_metadata_is_a_422_response() {
    let map = headers(&[("x-request-id", "upstream")]);
    let raw = RawResponse {
        headers: &map,
        body: br#"{"candidates":[{"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP"}]}"#,
    };
    let failure = transform_response(&raw, &request(REQUESTED_MODEL), &mut fixed_id).unwrap_err();
    assert_eq!(failure.status_code, 422);
    assert_eq!(
        failure
            .headers
            .as_ref()
            .and_then(|headers| headers.get("x-request-id"))
            .map(String::as_str),
        Some("upstream")
    );
}

#[test]
fn a_malformed_body_is_a_422_response() {
    let map = headers(&[("x-request-id", "upstream")]);
    let raw = RawResponse {
        headers: &map,
        body: b"{",
    };
    let failure = transform_response(&raw, &request(REQUESTED_MODEL), &mut fixed_id).unwrap_err();
    assert_eq!(failure.status_code, 422);
    assert!(failure.message.contains("malformed"));
    assert_eq!(
        failure
            .headers
            .as_ref()
            .and_then(|headers| headers.get("x-request-id"))
            .map(String::as_str),
        Some("upstream")
    );
}

#[test]
fn transform_response_reads_the_service_tier_and_served_model() {
    let map = headers(&[(SERVICE_TIER_HEADER, "STANDARD")]);
    let source = text_reply(r#","modelVersion":"served-model""#);
    let raw = RawResponse {
        headers: &map,
        body: source.as_bytes(),
    };
    let response = transform_response(&raw, &request(REQUESTED_MODEL), &mut fixed_id).unwrap();
    assert_eq!(response.model, "served-model");
    assert_eq!(response.id, GENERATED_ID);
    assert_eq!(response.service_tier.as_deref(), Some("default"));
}

#[test]
fn the_generated_id_is_drawn_before_the_tool_call_ids() {
    let second = Uuid::parse_str("fedcba9876543210fedcba9876543210").unwrap();
    let mut values = vec![Uuid::parse_str(GENERATED).unwrap(), second];
    let mut ids = move || values.remove(0);
    let body = decode_generate_content(
        br#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"f","args":{}}}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}"#,
    )
    .unwrap();
    let response = assemble_complete_response(
        &body,
        &HeaderMap::new(),
        &request(REQUESTED_MODEL),
        &mut ids,
    )
    .unwrap();
    assert_eq!(response.id, GENERATED_ID);
    let Choices::Complete(choices) = &response.choices else {
        panic!("a complete reply carries complete choices");
    };
    let call = choices[0]
        .message
        .tool_calls
        .as_ref()
        .unwrap()
        .first()
        .unwrap();
    assert_eq!(call.id, "call_fedcba9876543210fedcba987654");
}

#[test]
fn grounding_counters_come_from_the_projected_candidates() {
    let response = complete(
        r#"{"candidates":[{"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP",
            "groundingMetadata":{"webSearchQueries":["q"]}}],
            "usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":1,
                "toolUsePromptTokenCount":6,"totalTokenCount":11}}"#,
    );
    let details = response.usage.prompt_tokens_details.unwrap();
    assert_eq!(details.web_search_requests, Some(1));
    assert_eq!(response.usage.prompt_tokens, 4);
}

#[test]
fn usage_grounding_detection_reads_candidates_the_choices_skip() {
    let response = complete(
        r#"{"candidates":[
            {"groundingMetadata":{"webSearchQueries":["q"]}},
            {"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":1,
                "toolUsePromptTokenCount":6,"totalTokenCount":11}}"#,
    );
    assert_eq!(response.usage.prompt_tokens, 4);
    let details = response.usage.prompt_tokens_details.unwrap();
    assert!(details.web_search_requests.is_none());
    assert!(response.metadata.unwrap().grounding_metadata.is_empty());
}

#[rstest]
fn replays_the_generate_content_corpus(#[files("fuzz/corpus/generate_content/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_generate_content(&bytes));
    let map = HeaderMap::new();
    let raw = RawResponse {
        headers: &map,
        body: &bytes,
    };
    drop(transform_response(
        &raw,
        &request(REQUESTED_MODEL),
        &mut fixed_id,
    ));
}
