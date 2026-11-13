#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde_json::json;

use super::chunk::chunk_parser;
use super::error::{OvhCloudError, get_error_class};
use super::request::{ChatRequestError, ChatTransform, complete_url, transform_request};
use crate::chat::ChatMessage;
use crate::chat_content::RefuseDownloads;
use crate::openai_base_config::Headers;

fn object(value: serde_json::Value) -> Object {
    serde_json::from_value(value).expect("a JSON object")
}

fn choices(parsed: &super::chunk::OvhCloudChunk) -> serde_json::Value {
    serde_json::to_value(&parsed.choices).expect("choices")
}

fn transform(optional_params: Object) -> Result<Object, ChatRequestError> {
    let messages: Vec<ChatMessage> =
        serde_json::from_value(json!([{"role": "user", "content": "hi"}])).expect("messages");
    let environment = BTreeMap::<String, String>::new();
    transform_request(&ChatTransform {
        model: "ovhcloud/Mistral-7B-Instruct-v0.3",
        messages: &messages,
        optional_params: &optional_params,
        environment: &environment,
        downloads: &RefuseDownloads,
    })
}

#[rstest]
#[case::default_base(
    None,
    "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1/chat/completions"
)]
#[case::given_base(
    Some("https://proxy.example/v1"),
    "https://proxy.example/v1/chat/completions"
)]
#[case::trailing_slashes_are_stripped(
    Some("https://proxy.example/v1///"),
    "https://proxy.example/v1/chat/completions"
)]
#[case::empty_base_stays_empty(Some(""), "/chat/completions")]
#[case::an_endpoint_suffix_is_not_removed(
    Some("https://proxy.example/v1/chat/completions/"),
    "https://proxy.example/v1/chat/completions/chat/completions"
)]
fn complete_url_uses_the_default_base_or_the_given_one(
    #[case] api_base: Option<&str>,
    #[case] url: &str,
) {
    assert_eq!(complete_url(api_base), url);
}

#[test]
fn transform_request_carries_the_parent_body_and_the_extra_body() {
    let body = transform(object(
        json!({"temperature": 0.5, "extra_body": {"top_k": 4}}),
    ))
    .expect("a body");
    let expected = object(json!({
        "model": "ovhcloud/Mistral-7B-Instruct-v0.3",
        "messages": [{"role": "user", "content": "hi"}],
        "temperature": 0.5,
        "top_k": 4,
    }));
    assert_eq!(body, expected);
}

#[rstest]
#[case::a_key_replaces_the_parent_body_value(
    json!({"extra_body": {"model": "over", "messages": [], "temperature": 1}}),
    json!({"model": "over", "messages": [], "temperature": 1})
)]
#[case::an_empty_map_changes_nothing(json!({"extra_body": {}}), json!({}))]
fn transform_request_lays_extra_body_over_the_parent_body(
    #[case] optional_params: serde_json::Value,
    #[case] expected: serde_json::Value,
) {
    let body = transform(object(optional_params)).expect("a body");
    let mut wanted = object(json!({
        "model": "ovhcloud/Mistral-7B-Instruct-v0.3",
        "messages": [{"role": "user", "content": "hi"}],
    }));
    wanted.extend(object(expected));
    assert_eq!(body, wanted);
    assert!(!body.contains_key("extra_body"));
}

#[rstest]
#[case::null(json!({"extra_body": null}))]
#[case::number(json!({"extra_body": 4}))]
#[case::text(json!({"extra_body": "x"}))]
#[case::list(json!({"extra_body": [1]}))]
fn transform_request_rejects_an_extra_body_that_is_not_an_object(
    #[case] optional_params: serde_json::Value,
) {
    assert!(matches!(
        transform(object(optional_params)),
        Err(ChatRequestError::ExtraBody)
    ));
}

#[test]
fn get_error_class_keeps_the_message_status_and_headers() {
    let headers = Headers::from([(String::from("x-request-id"), String::from("req-1"))]);
    let error = get_error_class("bad request", 422, &headers);
    assert_eq!(
        error,
        OvhCloudError {
            message: String::from("bad request"),
            status_code: Some(422),
            headers,
        }
    );
}

#[test]
fn chunk_parser_builds_the_chunk_with_its_choices_and_usage() {
    let chunk = object(json!({
        "id": "chatcmpl-1",
        "created": 17,
        "model": "Mistral-7B-Instruct-v0.3",
        "choices": [
            {"index": 0, "delta": {"content": "hi"}, "finish_reason": null},
            {"index": 1, "delta": {"reasoning": "why"}, "finish_reason": "stop"},
        ],
        "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4},
    }));
    let parsed = chunk_parser(&chunk).expect("a chunk");
    assert_eq!(parsed.id, "chatcmpl-1");
    assert_eq!(parsed.object, "chat.completion.chunk");
    assert_eq!(parsed.created, 17);
    assert_eq!(parsed.model.as_deref(), Some("Mistral-7B-Instruct-v0.3"));
    assert_eq!(
        parsed.usage,
        Some(Json::Object(object(
            json!({"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4})
        )))
    );
    assert_eq!(
        choices(&parsed),
        json!([
            {"index": 0, "delta": {"content": "hi"}, "finish_reason": null},
            {"index": 1, "delta": {"reasoning": "why", "reasoning_content": "why"}, "finish_reason": "stop"},
        ])
    );
}

#[rstest]
#[case::reasoning_is_copied(
    json!({"index": 0, "delta": {"reasoning": "why"}}),
    json!({"index": 0, "delta": {"reasoning": "why", "reasoning_content": "why"}})
)]
#[case::an_existing_reasoning_content_wins(
    json!({"index": 0, "delta": {"reasoning": "new", "reasoning_content": "old"}}),
    json!({"index": 0, "delta": {"reasoning": "new", "reasoning_content": "old"}})
)]
#[case::a_null_reasoning_content_is_replaced(
    json!({"index": 0, "delta": {"reasoning": "new", "reasoning_content": null}}),
    json!({"index": 0, "delta": {"reasoning": "new", "reasoning_content": "new"}})
)]
#[case::a_null_reasoning_copies_nothing(
    json!({"index": 0, "delta": {"reasoning": null, "reasoning_content": null}}),
    json!({"index": 0, "delta": {"reasoning": null, "reasoning_content": null}})
)]
#[case::no_reasoning_key_copies_nothing(
    json!({"index": 0, "delta": {"content": "x"}}),
    json!({"index": 0, "delta": {"content": "x"}})
)]
#[case::a_choice_without_a_delta_is_kept_as_is(
    json!({"index": 0, "finish_reason": "stop"}),
    json!({"index": 0, "finish_reason": "stop"})
)]
fn chunk_parser_normalizes_reasoning_per_choice(
    #[case] choice: serde_json::Value,
    #[case] expected: serde_json::Value,
) {
    let chunk = object(json!({"id": "a", "created": 1, "model": "m", "choices": [choice]}));
    let parsed = chunk_parser(&chunk).expect("a chunk");
    assert_eq!(choices(&parsed), json!([expected]));
}

#[rstest]
#[case::missing_id(json!({"created": 1, "model": "m", "choices": []}))]
#[case::missing_created(json!({"id": "a", "model": "m", "choices": []}))]
#[case::missing_model(json!({"id": "a", "created": 1, "choices": []}))]
#[case::missing_choices(json!({"id": "a", "created": 1, "model": "m"}))]
fn chunk_parser_reports_a_missing_required_key_with_status_400(#[case] source: serde_json::Value) {
    let error = chunk_parser(&object(source)).expect_err("a KeyError");
    assert_eq!(error.status_code, Some(400));
    assert_eq!(
        error.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn chunk_parser_rejects_choices_that_are_not_a_list() {
    let chunk = object(json!({"id": "a", "created": 1, "model": "m", "choices": {}}));
    let error = chunk_parser(&chunk).expect_err("an error");
    assert_eq!(error.status_code, Some(400));
    assert!(error.message.contains("choices"));
}

#[rstest]
#[case::wrong_id(json!({"id": 4, "created": 1, "model": "m", "choices": []}))]
#[case::wrong_created(json!({"id": "a", "created": "1", "model": "m", "choices": []}))]
#[case::wrong_model(json!({"id": "a", "created": 1, "model": 4, "choices": []}))]
#[case::choice_not_an_object(json!({"id": "a", "created": 1, "model": "m", "choices": [4]}))]
#[case::delta_not_an_object(json!({"id": "a", "created": 1, "model": "m", "choices": [{"delta": 4}]}))]
fn chunk_parser_reports_a_wrong_shape_without_inventing_a_status(
    #[case] source: serde_json::Value,
) {
    let error = chunk_parser(&object(source)).expect_err("an error");
    assert_eq!(error.status_code, None);
}

#[test]
fn chunk_parser_keeps_a_null_model_and_an_absent_usage() {
    let chunk = object(json!({"id": "a", "created": 1, "model": null, "choices": []}));
    let parsed = chunk_parser(&chunk).expect("a chunk");
    assert_eq!(parsed.model, None);
    assert_eq!(parsed.usage, None);
}

#[test]
fn chunk_parser_keeps_a_json_null_usage() {
    let chunk =
        object(json!({"id": "a", "created": 1, "model": "m", "choices": [], "usage": null}));
    let parsed = chunk_parser(&chunk).expect("a chunk");
    assert_eq!(parsed.usage, Some(Json::Null));
}

#[test]
fn chunk_parser_reads_an_error_chunk_before_the_choices() {
    let chunk = object(json!({"error": {"message": "boom", "code": 429}}));
    let error = chunk_parser(&chunk).expect_err("an error chunk");
    assert_eq!(error.message, "OVHCloud Error: boom");
    assert_eq!(error.status_code, Some(429));
    assert_eq!(
        error.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[rstest]
#[case::missing_message(
    json!({"error": {"code": 429}}),
    Some(429),
    "OVHCloud Error: Unknown error"
)]
#[case::null_message(
    json!({"error": {"message": null}}),
    Some(400),
    "OVHCloud Error: Unknown error"
)]
#[case::missing_code(json!({"error": {"message": "one"}}), Some(400), "OVHCloud Error: one")]
#[case::null_code(json!({"error": {"message": "one", "code": null}}), None, "OVHCloud Error: one")]
fn chunk_parser_defaults_the_error_message_and_status(
    #[case] source: serde_json::Value,
    #[case] status: Option<u16>,
    #[case] message: &str,
) {
    let error = chunk_parser(&object(source)).expect_err("an error chunk");
    assert_eq!(error.status_code, status);
    assert_eq!(error.message, message);
}

#[rstest]
#[case::error_not_an_object(json!({"error": 4}))]
#[case::error_code_not_a_number(json!({"error": {"message": "one", "code": "429"}}))]
#[case::error_code_out_of_range(json!({"error": {"message": "one", "code": 70000}}))]
#[case::error_message_not_text(json!({"error": {"message": 4}}))]
fn chunk_parser_reports_an_error_shape_without_a_status(#[case] source: serde_json::Value) {
    let error = chunk_parser(&object(source)).expect_err("an error");
    assert_eq!(error.status_code, None);
}
