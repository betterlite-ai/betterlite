#![cfg(test)]

use serde_json::{Value, json};

use super::support::{Fixture, headers, messages, object};
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_chat::request::{CompletionInput, CompletionRequest, completion_request};

struct Case {
    model: &'static str,
    message_json: &'static str,
    api_base: Option<&'static str>,
    env_base: Option<&'static str>,
    params: &'static str,
    caller_header_json: &'static str,
    uploads: &'static [(&'static str, &'static str)],
}

fn default_case() -> Case {
    Case {
        model: "gigachat/GigaChat",
        message_json: r#"[{"role": "user", "content": "hi"}]"#,
        api_base: None,
        env_base: None,
        params: "{}",
        caller_header_json: "{}",
        uploads: &[],
    }
}

async fn completion(request_case: &Case) -> Result<CompletionRequest, GigaChatError> {
    let fixture = Fixture::issuing("token-1", request_case.uploads);
    let parsed = messages(request_case.message_json);
    let params = object(request_case.params);
    let caller = headers(request_case.caller_header_json);
    completion_request(
        &fixture.config(),
        &CompletionInput {
            model: request_case.model,
            messages: &parsed,
            api_base: request_case.api_base,
            env_base: request_case.env_base,
            api_key: None,
            credentials: None,
            fallback_key: None,
            caller_headers: &caller,
            optional_params: &params,
        },
    )
    .await
}

#[tokio::test]
async fn the_url_appends_chat_completions_to_the_default_base() {
    let request = completion(&default_case()).await.unwrap();
    assert_eq!(
        request.url,
        "https://gigachat.devices.sberbank.ru/api/v1/chat/completions"
    );
}

#[tokio::test]
async fn the_environment_base_is_used_without_an_api_base() {
    let mut request_case = default_case();
    request_case.env_base = Some("https://env.example/v1");
    let request = completion(&request_case).await.unwrap();
    assert_eq!(request.url, "https://env.example/v1/chat/completions");
}

#[tokio::test]
async fn the_api_base_wins_over_the_environment_base() {
    let mut request_case = default_case();
    request_case.api_base = Some("https://api.example/v2");
    request_case.env_base = Some("https://env.example/v1");
    let request = completion(&request_case).await.unwrap();
    assert_eq!(request.url, "https://api.example/v2/chat/completions");
}

#[tokio::test]
async fn only_the_gigachat_slash_prefix_is_removed_from_the_model() {
    let mut request_case = default_case();
    request_case.model = "gigachat/gigachat-pro";
    let request = completion(&request_case).await.unwrap();
    assert_eq!(
        json!(request.body).get("model"),
        Some(&Value::from("gigachat-pro"))
    );
}

#[tokio::test]
async fn the_body_carries_the_mapped_options_that_are_present() {
    let mut request_case = default_case();
    request_case.params = r#"{"temperature": 0.4, "top_p": 0.9, "max_tokens": 8, "stream": true, "repetition_penalty": 1.1, "profanity_check": false, "functions": [{"name": "lookup"}], "function_call": "auto"}"#;
    let request = completion(&request_case).await.unwrap();
    assert_eq!(
        json!(request.body),
        Value::Object(object(
            r#"{"model": "GigaChat", "messages": [{"role": "user", "content": "hi"}], "temperature": 0.4, "top_p": 0.9, "max_tokens": 8, "stream": true, "repetition_penalty": 1.1, "profanity_check": false, "functions": [{"name": "lookup"}], "function_call": "auto"}"#
        ))
    );
}

#[tokio::test]
async fn the_structured_output_flag_stays_out_of_the_body() {
    let mut request_case = default_case();
    request_case.params = r#"{"functions": [{"name": "city"}], "function_call": {"name": "city"}, "_structured_output": true}"#;
    let body = json!(completion(&request_case).await.unwrap().body);
    assert_eq!(
        body.get("function_call"),
        Some(&Value::Object(object(r#"{"name": "city"}"#)))
    );
    assert!(body.get("_structured_output").is_none());
}

#[tokio::test]
async fn the_headers_carry_the_bearer_token() {
    let request = completion(&default_case()).await.unwrap();
    assert_eq!(
        request.headers.get("Authorization"),
        Some(&String::from("Bearer token-1"))
    );
}

#[tokio::test]
async fn the_debug_text_names_the_headers_but_hides_the_token() {
    let request = completion(&default_case()).await.unwrap();
    let text = format!("{request:?}");
    assert!(text.contains("Authorization"));
    assert!(!text.contains("token-1"));
}

#[tokio::test]
async fn the_request_uploads_images_before_it_builds_the_body() {
    let mut request_case = default_case();
    request_case.message_json = r#"[{"role": "user", "content": [{"type": "text", "text": "look"}, {"type": "image_url", "image_url": {"url": "https://images.example/x.png"}}]}]"#;
    request_case.uploads = &[("https://images.example/x.png", "file-9")];
    let fixture = Fixture::issuing("token-1", request_case.uploads);
    let parsed = messages(request_case.message_json);
    let params = object(request_case.params);
    let caller = headers(request_case.caller_header_json);
    let request = completion_request(
        &fixture.config(),
        &CompletionInput {
            model: request_case.model,
            messages: &parsed,
            api_base: request_case.api_base,
            env_base: request_case.env_base,
            api_key: None,
            credentials: None,
            fallback_key: None,
            caller_headers: &caller,
            optional_params: &params,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        json!(request.body).get("messages"),
        Some(&Value::Array(vec![Value::Object(object(
            r#"{"role": "user", "content": "look", "attachments": ["file-9"]}"#
        ))]))
    );
    assert_eq!(
        fixture.uploader.uploads(),
        vec![String::from("https://images.example/x.png")]
    );
}

#[tokio::test]
async fn a_failed_token_stops_the_request_before_the_url_is_built() {
    let fixture = Fixture::failing(500);
    let parsed = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let params = object("{}");
    let caller = headers("{}");
    let error = completion_request(
        &fixture.config(),
        &CompletionInput {
            model: "gigachat/GigaChat",
            messages: &parsed,
            api_base: None,
            env_base: None,
            api_key: None,
            credentials: None,
            fallback_key: None,
            caller_headers: &caller,
            optional_params: &params,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code, 500);
}
