#![cfg(test)]

use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::answer::{chat_completion_body, text_completion_body, text_completion_response};
use super::{ChatEndpoint, chat_endpoint, chat_request, text_endpoint_model, text_request};
use crate::chat::ChatRequest;
use crate::chat::deployment::{ChatDeployment, OpenAiDefaults, Provider};
use crate::text_completion_request::CompletionRequest;
use crate::text_completion_response::CompletionResponse;

fn deployment(model: &str) -> ChatDeployment {
    let listed = ModelDeployment {
        model_name: String::from("g"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(model.to_owned()),
            api_base: Some(String::from("http://127.0.0.1:9/v1")),
            api_key: Some(SecretString::from("configured-key")),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo::default(),
    };
    ChatDeployment::resolve(&listed, &OpenAiDefaults::default()).expect("deployment")
}

// a provider whose identity resolution refused it, the way a text-completion prefix resolves today
fn unsupported(model: &str) -> ChatDeployment {
    let mut deployment = deployment("openai/gpt-4o");
    model.clone_into(&mut deployment.litellm_model);
    deployment.provider = Provider::Unsupported {
        model: model.to_owned(),
    };
    deployment
}

fn chat(body: &str) -> ChatRequest {
    ChatRequest::parse(body.as_bytes()).expect("chat request")
}

fn completion(body: &str) -> CompletionRequest {
    CompletionRequest::parse(body.as_bytes()).expect("completion request")
}

fn response(body: &str) -> CompletionResponse {
    serde_json::from_str(body).expect("completion response")
}

#[test]
fn an_ordinary_chat_model_stays_on_the_chat_endpoint() {
    assert_eq!(
        chat_endpoint(&deployment("openai/gpt-4o")),
        ChatEndpoint::Chat
    );
    assert_eq!(text_endpoint_model(&deployment("openai/gpt-4o")), None);
}

#[test]
fn a_text_completion_prefix_selects_the_converting_text_endpoint() {
    let prefixed = unsupported("text-completion-openai/oracle-text");
    assert_eq!(
        chat_endpoint(&prefixed),
        ChatEndpoint::Text { convert: true }
    );
    assert_eq!(
        text_endpoint_model(&prefixed),
        Some(String::from("oracle-text"))
    );
    let codestral = unsupported("text-completion-codestral/oracle-text");
    assert_eq!(
        chat_endpoint(&codestral),
        ChatEndpoint::Text { convert: true }
    );
}

#[test]
fn the_fine_tuned_models_select_the_raw_text_endpoint() {
    for model in ["openai/ft:babbage-002", "openai/ft:davinci-002"] {
        let resolved = deployment(model);
        assert_eq!(
            chat_endpoint(&resolved),
            ChatEndpoint::Text { convert: false }
        );
        assert_eq!(
            text_endpoint_model(&resolved),
            Some(model.trim_start_matches("openai/").to_owned())
        );
    }
    let unresolved = unsupported("openai/ft:babbage-002");
    assert_eq!(
        text_endpoint_model(&unresolved),
        Some(String::from("ft:babbage-002"))
    );
}

#[test]
fn a_stream_carries_the_usage_option_the_proxy_injects() {
    let request =
        chat(r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"stream":true}"#);
    let built = text_request(&deployment("gpt-4o"), &request, "gpt-4o").expect("request");
    assert_eq!(built.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        built.messages,
        Some(vec![json!({"role": "user", "content": "hi"})])
    );
    assert_eq!(
        built.optional_params.get("stream"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        built.optional_params.get("stream_options"),
        Some(&json!({"include_usage": true}))
    );
    assert!(!built.optional_params.contains_key("messages"));
    assert!(!built.optional_params.contains_key("model"));
}

#[test]
fn a_client_usage_choice_is_kept_and_a_non_stream_sends_an_explicit_false() {
    let kept = chat(
        r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":false}}"#,
    );
    let built = text_request(&deployment("gpt-4o"), &kept, "gpt-4o").expect("request");
    assert_eq!(
        built.optional_params.get("stream_options"),
        Some(&json!({"include_usage": false}))
    );
    let buffered = chat(r#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#);
    let built = text_request(&deployment("gpt-4o"), &buffered, "gpt-4o").expect("request");
    assert_eq!(
        built.optional_params.get("stream"),
        Some(&Value::Bool(false))
    );
    assert!(!built.optional_params.contains_key("stream_options"));
    assert_eq!(built.optional_params.get("model"), None);
}

#[test]
fn control_kwargs_and_internal_keys_stay_off_the_provider_body() {
    let request = chat(
        r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"stream_chunk_size":5,"_hidden":true,"temperature":0.5}"#,
    );
    let built = text_request(&deployment("gpt-4o"), &request, "gpt-4o").expect("request");
    for name in ["stream_chunk_size", "_hidden"] {
        assert!(!built.optional_params.contains_key(name), "{name} leaked");
    }
    assert_eq!(built.optional_params.get("temperature"), Some(&json!(0.5)));
}

#[test]
fn one_user_message_per_prompt_string() {
    let single = chat_request(&completion(r#"{"model":"m","prompt":"hello"}"#)).expect("chat");
    assert_eq!(single.messages.len(), 1);
    assert_eq!(single.messages[0].role, crate::chat::Role::User);
    assert_eq!(
        serde_json::to_value(&single.messages[0].content).expect("content"),
        json!("hello")
    );
    let many = chat_request(&completion(r#"{"model":"m","prompt":["a","b"]}"#)).expect("chat");
    assert_eq!(many.messages.len(), 2);
    assert_eq!(many.model.as_deref(), Some("m"));
}

#[test]
fn a_token_prompt_and_an_unmapped_prompt_fail_before_the_call() {
    let tokens = chat_request(&completion(r#"{"model":"m","prompt":[1,2]}"#)).expect_err("tokens");
    assert_eq!(tokens.status, 400);
    assert_eq!(tokens.error_type.as_deref(), Some("invalid_request_error"));
    let empty = completion(r#"{"model":"m","prompt":[]}"#);
    let failure = chat_request(&empty).expect_err("unmapped");
    assert_eq!(failure.status, 500);
    assert_eq!(failure.error_type, None);
    assert_eq!(
        failure.message,
        "Unmapped prompt format. Your prompt is neither a list of strings nor a string."
    );
}

#[test]
fn functions_keep_their_place_in_the_chat_passthrough() {
    let request = completion(
        r#"{"model":"m","prompt":"hi","functions":[{"name":"f","parameters":{"type":"object"}}]}"#,
    );
    let chat = chat_request(&request).expect("chat");
    assert!(chat.passthrough.contains_key("functions"));
}

#[test]
fn the_chat_route_drops_the_nulls_of_a_converted_text_answer() {
    let text = response(
        r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"upstream","choices":[{"text":"Hello","index":0,"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null},"system_fingerprint":null}"#,
    );
    let body = chat_completion_body(&text, 1794865546, Some("client-model")).expect("body");
    let value: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        value,
        json!({
            "id": "cmpl-1",
            "created": 1794865546,
            "model": "client-model",
            "object": "chat.completion",
            "choices": [{
                "finish_reason": "stop",
                "index": 0,
                "message": {"content": "Hello", "role": "assistant"}
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
        })
    );
}

#[test]
fn the_fine_tuned_answer_keeps_its_text_shape_with_the_client_model() {
    let text = response(
        r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"upstream","choices":[{"text":"Hello","index":0,"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5},"system_fingerprint":null}"#,
    );
    let body = text_completion_body(&text, Some("ft-babbage")).expect("body");
    let value: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(value.get("object"), Some(&json!("text_completion")));
    assert_eq!(value.get("model"), Some(&json!("ft-babbage")));
    assert_eq!(value.get("system_fingerprint"), None);
    assert_eq!(value.pointer("/choices/0/text"), Some(&json!("Hello")));
}

#[test]
fn a_chat_answer_becomes_the_text_response_the_route_writes() {
    let chat: crate::chat::response::ChatResponse = serde_json::from_str(
        r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
    )
    .expect("chat response");
    let converted = text_completion_response(chat);
    assert_eq!(converted.object, "text_completion");
    assert_eq!(converted.created, 1_700_000_000);
    assert_eq!(converted.choices[0].text.as_deref(), Some("Hello"));
    assert_eq!(converted.choices[0].logprobs, None);
    assert_eq!(
        converted.usage.as_ref().map(|usage| usage.total_tokens),
        Some(5)
    );
    assert_eq!(converted.system_fingerprint, None);
}
