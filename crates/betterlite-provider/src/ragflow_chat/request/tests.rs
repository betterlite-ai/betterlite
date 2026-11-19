#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;

use super::{RequestInput, transform_request};
use crate::ragflow_chat::error::RagflowError;
use crate::ragflow_chat::tests::support::{messages, object};

fn body(model: &str, messages: &[Message], options: &Object, params: &Object) -> Object {
    transform_request(&RequestInput {
        model,
        messages,
        optional_params: options,
        litellm_params: params,
    })
    .expect("the fixture request builds")
}

fn text(value: &str) -> Json {
    Json::Text(String::from(value))
}

fn only_message(body: &Object) -> &Object {
    let Some(Json::List(entries)) = body.get("messages") else {
        panic!("the body holds a message list");
    };
    let Some(Json::Object(first)) = entries.first() else {
        panic!("the body holds one message object");
    };
    first
}

#[test]
fn the_parsed_model_name_goes_into_the_body() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = body(
        "ragflow/chat/abc/openai/gpt-4o",
        &messages,
        &object("{}"),
        &object("{}"),
    );
    assert_eq!(body.get("model"), Some(&text("openai/gpt-4o")));
    let message = only_message(&body);
    assert_eq!(message.get("role"), Some(&text("user")));
    assert_eq!(message.get("content"), Some(&text("hi")));
}

#[test]
fn a_stored_model_name_wins_over_the_parsed_one() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(r#"{"_ragflow_actual_model": "stored-model"}"#);
    let body = body("ragflow/chat/abc/gpt-4o", &messages, &object("{}"), &params);
    assert_eq!(body.get("model"), Some(&text("stored-model")));
}

#[test]
fn a_null_stored_model_falls_back_to_the_parsed_name() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(r#"{"_ragflow_actual_model": null}"#);
    let body = body("ragflow/agent/abc/glm-4", &messages, &object("{}"), &params);
    assert_eq!(body.get("model"), Some(&text("glm-4")));
}

#[test]
fn a_model_that_does_not_parse_goes_through_whole() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = body("other/chat", &messages, &object("{}"), &object("{}"));
    assert_eq!(body.get("model"), Some(&text("other/chat")));
}

#[test]
fn a_non_string_stored_model_is_refused() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(r#"{"_ragflow_actual_model": 5}"#);
    let error = transform_request(&RequestInput {
        model: "ragflow/chat/abc/m",
        messages: &messages,
        optional_params: &object("{}"),
        litellm_params: &params,
    })
    .expect_err("a non-string stored model is refused");
    assert!(matches!(error, RagflowError::FieldType { .. }));
    assert_eq!(
        error.to_string(),
        "the _ragflow_actual_model value must be a string"
    );
}

#[test]
fn the_body_is_the_optional_params_with_the_model_and_messages() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let options = object(r#"{"stream": true, "max_retries": 3, "tool_choice": "auto"}"#);
    let body = body(
        "ragflow/chat/abc/openai/gpt-4o",
        &messages,
        &options,
        &object("{}"),
    );
    assert_eq!(
        Json::Object(body),
        Json::Object(object(
            r#"{"stream": true, "max_retries": 3, "tool_choice": "auto", "model": "openai/gpt-4o",
                "messages": [{"role": "user", "content": "hi"}]}"#
        ))
    );
}

#[test]
fn the_messages_are_sent_unchanged() {
    let wire = r#"[
        {"role": "user", "content": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}]},
        {"role": "tool", "tool_call_id": "call-1", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/cat.png", "format": "png"}}]}
    ]"#;
    let messages = messages(wire);
    let body = body(
        "ragflow/chat/abc/m",
        &messages,
        &object("{}"),
        &object("{}"),
    );
    assert_eq!(
        body.get("messages"),
        Some(&serde_json::from_str::<Json>(wire).expect("the fixture messages are JSON"))
    );
}
