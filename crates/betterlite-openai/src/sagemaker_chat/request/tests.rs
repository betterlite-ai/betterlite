#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::Object as Body;
use serde_json::{Value, json};

use super::{
    HAS_CUSTOM_STREAM_WRAPPER, SUPPORTS_STREAM_PARAM_IN_BODY, TransformInput, get_error_class,
    map_openai_params, supported_openai_params, transform_request, validate_environment,
};
use crate::chat::ChatMessage;
use crate::openai_base_config::{CatalogCapabilities, Headers, Object};
use crate::sagemaker_chat::nova;

fn messages() -> Vec<ChatMessage> {
    serde_json::from_value(json!([{"role": "user", "content": "hi"}])).expect("the messages")
}

fn options(source: &str) -> Body {
    serde_json::from_str(source).expect("the options")
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn request<'request>(
    messages: &'request [ChatMessage],
    options: &'request Body,
    hf_model_name: Option<&'request str>,
) -> TransformInput<'request> {
    TransformInput {
        model: "endpoint",
        messages,
        optional_params: options,
        hf_model_name,
    }
}

fn keys(body: &Body) -> Vec<&str> {
    body.keys().map(String::as_str).collect()
}

#[test]
fn the_inference_component_header_is_added_for_a_string_model_id() {
    let headers = Headers::from([(
        String::from("Content-Type"),
        String::from("application/json"),
    )]);
    let merged = validate_environment(headers, &options(r#"{"model_id": "component"}"#));
    assert_eq!(
        merged.get("X-Amzn-SageMaker-Inference-Component"),
        Some(&String::from("component"))
    );
    assert_eq!(
        merged.get("Content-Type"),
        Some(&String::from("application/json"))
    );
}

#[test]
fn headers_pass_through_without_a_string_model_id() {
    let headers = Headers::from([(String::from("x-fixture"), String::from("kept"))]);
    assert_eq!(
        validate_environment(headers.clone(), &options("{}")),
        headers
    );
    assert_eq!(
        validate_environment(headers.clone(), &options(r#"{"model_id": 3}"#)),
        headers
    );
    assert_eq!(
        validate_environment(headers.clone(), &options(r#"{"model_id": null}"#)),
        headers
    );
}

#[test]
fn the_body_carries_the_model_the_messages_and_the_options() {
    let messages = messages();
    let options = options(r#"{"temperature": 0.7}"#);
    let built = transform_request(&request(&messages, &options, None)).expect("a body");
    assert_eq!(keys(&built), ["model", "messages", "temperature"]);
    assert_eq!(built.get("model"), Some(&json!("endpoint")));
}

#[test]
fn the_hf_model_name_replaces_the_model_and_keeps_its_place() {
    let messages = messages();
    let options = options(r#"{"temperature": 0.7}"#);
    let built =
        transform_request(&request(&messages, &options, Some("hf-endpoint"))).expect("a body");
    assert_eq!(keys(&built), ["model", "messages", "temperature"]);
    assert_eq!(built.get("model"), Some(&json!("hf-endpoint")));
}

#[test]
fn nova_drops_the_model_from_the_body() {
    let messages = messages();
    let options = options(r#"{"temperature": 0.7}"#);
    let built = nova::transform_request(&request(&messages, &options, Some("hf-endpoint")))
        .expect("a body");
    assert_eq!(keys(&built), ["messages", "temperature"]);
}

#[test]
fn nova_appends_its_options_to_the_chat_list() {
    let params = nova::supported_openai_params("endpoint", &catalog());
    assert!(params.ends_with(&[
        "top_k",
        "reasoning_effort",
        "allowed_token_ids",
        "truncate_prompt_tokens"
    ]));
}

#[test]
fn the_chat_list_leaves_out_the_exact_openai_extensions() {
    let params = supported_openai_params("endpoint", &catalog());
    assert!(params.contains(&"response_format"));
    assert!(!params.contains(&"user"));
    assert!(!params.contains(&"reasoning_effort"));
    assert!(!params.contains(&"top_k"));
}

#[test]
fn the_stream_flags_match_the_pinned_config() {
    assert!(HAS_CUSTOM_STREAM_WRAPPER);
    assert!(!SUPPORTS_STREAM_PARAM_IN_BODY);
    assert!(nova::HAS_CUSTOM_STREAM_WRAPPER);
    assert!(nova::SUPPORTS_STREAM_PARAM_IN_BODY);
}

#[test]
fn map_openai_params_keeps_only_the_supported_parameters() {
    let mut existing = Object::new();
    let requested = serde_json::from_str(r#"{"temperature": 0.2, "unsupported": 1}"#)
        .expect("the requested options");
    map_openai_params("endpoint", &requested, &mut existing, &catalog());
    assert_eq!(Value::Object(existing), json!({"temperature": 0.2}));
}

#[test]
fn get_error_class_keeps_the_status_message_and_headers() {
    let headers = Headers::from([(String::from("x-fixture"), String::from("kept"))]);
    let error = get_error_class("boom", 418, Some(headers.clone()));
    assert_eq!(error.status_code, 418);
    assert_eq!(error.message, "boom");
    assert_eq!(error.headers, Some(headers));
}
