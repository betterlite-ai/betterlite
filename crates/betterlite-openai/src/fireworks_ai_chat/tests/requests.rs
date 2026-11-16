use std::collections::BTreeMap;

use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde_json::{Value, json};

use super::super::request::{RequestInput, transform_request};
use super::{message, object, prompt_json, request_body};
use crate::chat_content::RefuseDownloads;

#[test]
fn transform_request_resolves_the_model_for_the_openai_transform() {
    let body = request_body("models/llama-v3", json!({"temperature": 0_i64}));
    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from(
            "accounts/fireworks/models/llama-v3"
        )))
    );
    assert_eq!(body.get("temperature"), Some(&prompt_json(json!(0_i64))));
}

#[test]
fn transform_request_resolves_a_provider_prefixed_model() {
    let body = request_body("fireworks_ai/accounts/fireworks/models/llama", json!({}));
    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from("accounts/fireworks/models/llama")))
    );
}

#[rstest]
#[case::stream_only(json!({"stream": true}))]
#[case::null_stream_options(json!({"stream": true, "stream_options": null}))]
#[case::empty_stream_options(json!({"stream": true, "stream_options": {}}))]
fn transform_request_defaults_stream_options_for_streaming(#[case] params: Value) {
    let body = request_body("models/llama", params);
    assert_eq!(
        body.get("stream_options"),
        Some(&prompt_json(json!({"include_usage": true})))
    );
}

#[test]
fn transform_request_keeps_or_adds_include_usage() {
    let kept = request_body(
        "models/llama",
        json!({"stream": true, "stream_options": {"include_usage": false, "foo": 1_i64}}),
    );
    assert_eq!(
        kept.get("stream_options"),
        Some(&prompt_json(json!({"include_usage": false, "foo": 1_i64})))
    );

    let added = request_body(
        "models/llama",
        json!({"stream": true, "stream_options": {"foo": 1_i64}}),
    );
    assert_eq!(
        added.get("stream_options"),
        Some(&prompt_json(json!({"foo": 1_i64, "include_usage": true})))
    );
}

#[test]
fn transform_request_leaves_stream_options_alone_without_stream() {
    let absent = request_body("models/llama", json!({}));
    assert!(absent.get("stream_options").is_none());

    let kept = request_body(
        "models/llama",
        json!({"stream": false, "stream_options": {"include_usage": false}}),
    );
    assert_eq!(
        kept.get("stream_options"),
        Some(&prompt_json(json!({"include_usage": false})))
    );

    let untouched = request_body("models/llama", json!({"stream_options": {"foo": 1_i64}}));
    assert_eq!(
        untouched.get("stream_options"),
        Some(&prompt_json(json!({"foo": 1_i64})))
    );
}

#[test]
fn transform_request_rejects_a_stream_options_that_is_not_an_object() {
    let messages = vec![message(json!({"role": "user", "content": "hi"}))];
    let optional_params = object(r#"{"stream": true, "stream_options": "yes"}"#);
    let environment = BTreeMap::new();
    let error = transform_request(&RequestInput {
        model: "models/llama",
        messages: &messages,
        optional_params: &optional_params,
        catalog: &Catalog::new(),
        environment: &environment,
        downloads: &RefuseDownloads,
    })
    .expect_err("stream_options");
    assert_eq!(error.to_string(), "stream_options must be an object");
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn transform_request_passes_cleaned_tools_to_the_body() {
    let body = request_body(
        "models/llama",
        json!({"tools": [{"type": "function", "function": {"name": "lookup", "strict": true, "parameters": {"type": "object"}}}]}),
    );
    let Some(Json::List(tools)) = body.get("tools") else {
        panic!("tools in the body");
    };
    let Json::Object(function) = &tools[0].as_object().expect("tool")["function"] else {
        panic!("function in the tool");
    };
    assert!(!function.contains_key("strict"));
}

#[test]
fn transform_request_leaves_a_null_tools_value_alone() {
    let body = request_body("models/llama", json!({"tools": null}));
    assert_eq!(body.get("tools"), Some(&Json::Null));
}
