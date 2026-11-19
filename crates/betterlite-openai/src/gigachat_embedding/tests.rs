#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::{
    EmbeddingRequest, EmbeddingRequestInput, GigaChatEmbeddingError, SUPPORTED_OPENAI_PARAMS,
    embedding_request, get_error_class, transform_response,
};
use crate::embedding::EmbeddingInput;
use crate::gigachat_chat::config::GigachatBearer;
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::GIGACHAT_BASE_URL;
use crate::openai_base_config::Headers;

struct ScriptedBearer {
    token: Option<&'static str>,
    status_code: u16,
    seen: Mutex<Vec<Option<String>>>,
}

impl ScriptedBearer {
    fn issuing(token: &'static str) -> Self {
        Self {
            token: Some(token),
            status_code: 200,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn failing(status_code: u16) -> Self {
        Self {
            token: None,
            status_code,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn credentials(&self) -> Vec<Option<String>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl GigachatBearer for ScriptedBearer {
    async fn access_token(
        &self,
        credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError> {
        self.seen
            .lock()
            .unwrap()
            .push(credentials.map(|value| value.expose_secret().to_owned()));
        self.token.map_or_else(
            || {
                Err(GigaChatError {
                    status_code: self.status_code,
                    message: String::from("no token"),
                    headers: None,
                    source: None,
                })
            },
            |token| Ok(SecretString::from(token)),
        )
    }
}

fn headers(text: &str) -> Headers {
    serde_json::from_str(text).unwrap()
}

fn input<'request>(
    model: &'request str,
    value: &'request EmbeddingInput,
    caller_headers: &'request Headers,
) -> EmbeddingRequestInput<'request> {
    EmbeddingRequestInput {
        model,
        input: value,
        api_base: None,
        env_base: None,
        credentials: None,
        caller_headers,
    }
}

fn body_text(request: &EmbeddingRequest) -> serde_json::Value {
    serde_json::to_value(&request.body).unwrap()
}

fn value_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn the_url_joins_the_api_base_with_the_embeddings_path() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let mut request_input = input("Embeddings", &value, &caller);
    request_input.api_base = Some("https://api.test/v1");
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(request.url, "https://api.test/v1/embeddings");
}

#[tokio::test]
async fn the_url_falls_back_to_the_environment_base() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let mut request_input = input("Embeddings", &value, &caller);
    request_input.env_base = Some("https://env.test/v1");
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(request.url, "https://env.test/v1/embeddings");
}

#[tokio::test]
async fn the_url_falls_back_to_the_default_base() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(request.url, format!("{GIGACHAT_BASE_URL}/embeddings"));
}

#[tokio::test]
async fn a_text_input_becomes_a_one_item_list() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(
        body_text(&request),
        value_of(r#"{"model": "Embeddings", "input": ["hi"]}"#)
    );
}

#[tokio::test]
async fn a_texts_input_stays_a_list() {
    let value = EmbeddingInput::Texts(vec![String::from("hi"), String::from("there")]);
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(
        body_text(&request),
        value_of(r#"{"model": "Embeddings", "input": ["hi", "there"]}"#)
    );
}

#[tokio::test]
async fn a_tokens_input_stays_a_list() {
    let value = EmbeddingInput::Tokens(vec![1, 2]);
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(
        body_text(&request),
        value_of(r#"{"model": "Embeddings", "input": [1, 2]}"#)
    );
}

#[tokio::test]
async fn a_token_lists_input_stays_nested() {
    let value = EmbeddingInput::TokenLists(vec![vec![1, 2], vec![3]]);
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(
        body_text(&request),
        value_of(r#"{"model": "Embeddings", "input": [[1, 2], [3]]}"#)
    );
}

#[tokio::test]
async fn one_model_prefix_is_removed() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("gigachat/Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(body_text(&request)["model"], json!("Embeddings"));
}

#[tokio::test]
async fn a_model_without_the_prefix_is_kept() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(body_text(&request)["model"], json!("Embeddings"));
}

#[tokio::test]
async fn the_model_prefix_is_removed_only_at_the_start() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("chat/gigachat/Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(
        body_text(&request)["model"],
        json!("chat/gigachat/Embeddings")
    );
}

#[tokio::test]
async fn a_repeated_model_prefix_loses_one_copy() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("gigachat/gigachat/Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("t"), &request_input)
        .await
        .unwrap();
    assert_eq!(body_text(&request)["model"], json!("gigachat/Embeddings"));
}

#[tokio::test]
async fn the_token_becomes_the_authorization_header() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("token-1"), &request_input)
        .await
        .unwrap();
    assert_eq!(
        request.headers,
        headers(r#"{"Authorization": "Bearer token-1", "Content-Type": "application/json"}"#)
    );
}

#[tokio::test]
async fn caller_headers_win_over_the_defaults() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = headers(
        r#"{"Content-Type": "text/plain", "Authorization": "Bearer caller", "X-Trace": "abc"}"#,
    );
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("token-1"), &request_input)
        .await
        .unwrap();
    assert_eq!(
        request.headers,
        headers(
            r#"{"Authorization": "Bearer caller", "Content-Type": "text/plain", "X-Trace": "abc"}"#
        )
    );
}

#[tokio::test]
async fn a_differently_cased_caller_header_stays_a_second_entry() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = headers(r#"{"content-type": "text/plain"}"#);
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("token-1"), &request_input)
        .await
        .unwrap();
    assert_eq!(
        request.headers,
        headers(
            r#"{"Authorization": "Bearer token-1", "Content-Type": "application/json", "content-type": "text/plain"}"#
        )
    );
}

#[tokio::test]
async fn a_failed_token_keeps_its_status() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let error = embedding_request(&ScriptedBearer::failing(401), &request_input)
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 401);
    assert_eq!(error.message, "no token");
    assert_eq!(error.headers, None);
}

#[tokio::test]
async fn a_set_credential_is_passed_to_the_bearer() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let credential = SecretString::from("client-id:secret");
    let mut request_input = input("Embeddings", &value, &caller);
    request_input.credentials = Some(&credential);
    let bearer = ScriptedBearer::issuing("t");
    embedding_request(&bearer, &request_input).await.unwrap();
    assert_eq!(
        bearer.credentials(),
        vec![Some(String::from("client-id:secret"))]
    );
}

#[tokio::test]
async fn an_empty_credential_is_not_passed_to_the_bearer() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let credential = SecretString::from("");
    let mut request_input = input("Embeddings", &value, &caller);
    request_input.credentials = Some(&credential);
    let bearer = ScriptedBearer::issuing("t");
    embedding_request(&bearer, &request_input).await.unwrap();
    assert_eq!(bearer.credentials(), vec![None]);
}

#[test]
fn no_openai_parameter_is_supported() {
    assert!(SUPPORTED_OPENAI_PARAMS.is_empty());
}

#[test]
fn the_item_usage_is_summed_into_the_response_usage() {
    let response = transform_response(
        br#"{"object":"list","model":"Embeddings","data":[{"object":"embedding","embedding":[1,2],"index":0,"usage":{"prompt_tokens":4}},{"object":"embedding","embedding":[3],"index":1,"usage":{"prompt_tokens":6}}],"usage":{"prompt_tokens":99,"total_tokens":99}}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.total_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 0);
    assert_eq!(
        serde_json::to_value(&response.data).unwrap(),
        value_of(
            r#"[{"object": "embedding", "embedding": [1, 2], "index": 0}, {"object": "embedding", "embedding": [3], "index": 1}]"#
        )
    );
}

#[test]
fn the_reply_usage_is_overwritten() {
    let response = transform_response(
        br#"{"object":"list","model":"m","data":[],"usage":{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17}}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
}

#[test]
fn a_reply_without_data_counts_zero() {
    let response = transform_response(br#"{"object":"list","model":"m"}"#).unwrap();
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
    assert!(response.data.is_empty());
}

#[test]
fn a_null_data_fails_with_500() {
    let error = transform_response(br#"{"object":"list","model":"m","data":null}"#).unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn an_empty_data_list_counts_zero() {
    let response = transform_response(br#"{"object":"list","model":"m","data":[]}"#).unwrap();
    assert!(response.data.is_empty());
    assert_eq!(response.usage.total_tokens, 0);
}

#[test]
fn the_reply_object_is_forced_to_list() {
    let response = transform_response(br#"{"object":"embeddings","model":"m","data":[]}"#).unwrap();
    assert_eq!(response.object, "list");
}

#[test]
fn the_reply_model_is_the_one_the_provider_sent() {
    let response =
        transform_response(br#"{"object":"list","model":"Embeddings","data":[]}"#).unwrap();
    assert_eq!(response.model, "Embeddings");
}

#[test]
fn an_item_without_usage_counts_zero() {
    let response = transform_response(
        br#"{"object":"list","model":"m","data":[{"object":"embedding","embedding":[1],"index":0}]}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 0);
}

#[test]
fn an_item_usage_without_prompt_tokens_counts_zero() {
    let response = transform_response(
        br#"{"object":"list","model":"m","data":[{"object":"embedding","embedding":[1],"index":0,"usage":{"completion_tokens":2}}]}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 0);
}

#[test]
fn an_item_usage_that_is_not_an_object_fails_with_500() {
    let error = transform_response(
        br#"{"object":"list","model":"m","data":[{"object":"embedding","embedding":[1],"index":0,"usage":"usage"}]}"#,
    )
    .unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn an_item_that_is_not_an_object_fails_with_500() {
    let error =
        transform_response(br#"{"object":"list","model":"m","data":["not an item"]}"#).unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn a_count_that_is_not_a_number_fails_with_500() {
    let error = transform_response(
        br#"{"object":"list","model":"m","data":[{"object":"embedding","embedding":[1],"index":0,"usage":{"prompt_tokens":"4"}}]}"#,
    )
    .unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn a_negative_count_fails_with_500() {
    let error = transform_response(
        br#"{"object":"list","model":"m","data":[{"object":"embedding","embedding":[1],"index":0,"usage":{"prompt_tokens":-1}}]}"#,
    )
    .unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn a_huge_item_count_saturates() {
    let response = transform_response(
        br#"{"object":"list","model":"m","data":[{"object":"embedding","embedding":[1],"index":0,"usage":{"prompt_tokens":4000000000}},{"object":"embedding","embedding":[2],"index":1,"usage":{"prompt_tokens":4000000000}}]}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, u32::MAX);
    assert_eq!(response.usage.total_tokens, u32::MAX);
}

#[test]
fn a_reply_that_is_not_json_fails_with_500() {
    let error = transform_response(b"nope").unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.source.is_some());
}

#[test]
fn a_reply_that_is_not_an_object_fails_with_500() {
    let error = transform_response(b"[]").unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn a_reply_without_a_model_fails_with_500() {
    let error = transform_response(br#"{"object":"list","data":[]}"#).unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn a_data_field_that_is_not_a_list_fails_with_500() {
    let error = transform_response(br#"{"object":"list","model":"m","data":"x"}"#).unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn an_item_without_an_embedding_fails_with_500() {
    let error = transform_response(
        br#"{"object":"list","model":"m","data":[{"object":"embedding","index":0}]}"#,
    )
    .unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[tokio::test]
async fn the_debug_output_hides_the_header_values() {
    let value = EmbeddingInput::Text(String::from("hi"));
    let caller = Headers::new();
    let request_input = input("Embeddings", &value, &caller);
    let request = embedding_request(&ScriptedBearer::issuing("token-1"), &request_input)
        .await
        .unwrap();
    let text = format!("{request:?}");
    assert!(text.contains("Authorization"));
    assert!(!text.contains("token-1"));
}

#[test]
fn an_error_class_carries_the_arguments() {
    let given = headers(r#"{"X-Request-Id": "r-1"}"#);
    let error = get_error_class("boom", 429, &given);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "boom");
    assert_eq!(error.headers, Some(given));
    assert!(error.source.is_none());
}

#[test]
fn an_embedding_error_keeps_the_chat_error_fields() {
    let error = GigaChatEmbeddingError::from(GigaChatError {
        status_code: 503,
        message: String::from("down"),
        headers: Some(headers(r#"{"Retry-After": "3"}"#)),
        source: None,
    });
    assert_eq!(error.status_code, 503);
    assert_eq!(error.message, "down");
    assert!(error.headers.is_some());
}

#[rstest]
fn replays_the_gigachat_embedding_reply_corpus(
    #[files("fuzz/corpus/gigachat_embedding_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _decoded = transform_response(&bytes);
}
