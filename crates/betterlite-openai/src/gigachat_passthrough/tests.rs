#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::{
    GigachatPassthroughConfig, GigachatPassthroughError, GigachatPassthroughLogged,
    validate_environment,
};
use crate::gigachat_chat::config::GigachatBearer;
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::GIGACHAT_BASE_URL;
use crate::openai_base_config::{Headers, Object};
use crate::passthrough::transformation::BasePassthroughConfig as _;
use crate::passthrough::transformation::types::{
    PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayCallType, RelayedResponse,
};

struct NoopLogging;

impl PassthroughLogging for NoopLogging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

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

    fn seen(&self) -> Vec<Option<String>> {
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

fn passthrough_config(
    api_base_env: Option<&str>,
    api_key_env: Option<&str>,
) -> GigachatPassthroughConfig {
    GigachatPassthroughConfig {
        api_base_env: api_base_env.map(str::to_owned),
        api_key_env: api_key_env.map(SecretString::from),
    }
}

fn url_of(
    config: &GigachatPassthroughConfig,
    api_base: Option<&str>,
    endpoint: &str,
    query: Option<&Object>,
) -> (Url, String) {
    let params = Object::new();
    let request = PassthroughUrlRequest {
        api_base,
        api_key: None,
        target: PassthroughTarget {
            model: "Embeddings",
            custom_llm_provider: "gigachat",
            endpoint,
        },
        request_query_params: query,
        litellm_params: &params,
    };
    config.get_complete_url(&request).expect("a complete url")
}

fn logged_value(
    config: &GigachatPassthroughConfig,
    endpoint: &str,
    model: &str,
    body: &str,
) -> Option<GigachatPassthroughLogged> {
    let target = PassthroughTarget {
        model,
        custom_llm_provider: "gigachat",
        endpoint,
    };
    let response = RelayedResponse {
        status_code: 200,
        body: body.as_bytes(),
    };
    config.logging_non_streaming_response(&target, &response, &Object::new(), &mut NoopLogging)
}

fn collected(
    config: &GigachatPassthroughConfig,
    chunks: &[&str],
) -> Option<GigachatPassthroughLogged> {
    let owned: Vec<String> = chunks.iter().map(|chunk| (*chunk).to_owned()).collect();
    let target = PassthroughTarget {
        model: "m",
        custom_llm_provider: "gigachat",
        endpoint: "v1/chat/completions",
    };
    config.handle_collected_chunks(&owned, &mut NoopLogging, &target)
}

fn chunk_texts(logged: Option<GigachatPassthroughLogged>) -> Vec<String> {
    let Some(GigachatPassthroughLogged::Chunks(chunks)) = logged else {
        panic!("expected collected chunks");
    };
    chunks.iter().map(|chunk| chunk.text.clone()).collect()
}

fn request_data(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

#[test]
fn is_streaming_request_reads_a_true_stream_field() {
    let data = request_data(r#"{"stream": true}"#);
    assert!(passthrough_config(None, None).is_streaming_request("v1/chat/completions", &data));
}

#[test]
fn a_false_stream_field_is_not_streaming() {
    let data = request_data(r#"{"stream": false}"#);
    assert!(!passthrough_config(None, None).is_streaming_request("v1/chat/completions", &data));
}

#[test]
fn an_absent_stream_field_is_not_streaming() {
    assert!(
        !passthrough_config(None, None).is_streaming_request("v1/chat/completions", &Object::new())
    );
}

#[test]
fn a_non_boolean_stream_field_is_not_streaming() {
    let data = request_data(r#"{"stream": "true"}"#);
    assert!(!passthrough_config(None, None).is_streaming_request("v1/chat/completions", &data));
}

#[test]
fn the_url_joins_the_base_and_the_endpoint() {
    let (url, base) = url_of(
        &passthrough_config(None, None),
        Some("https://api.test/v1"),
        "/chat/completions",
        None,
    );
    assert_eq!(url.as_str(), "https://api.test/v1/chat/completions");
    assert_eq!(base, "https://api.test/v1");
}

#[test]
fn every_leading_slash_is_removed() {
    let (url, _) = url_of(
        &passthrough_config(None, None),
        Some("https://api.test/v1"),
        "//chat",
        None,
    );
    assert_eq!(url.as_str(), "https://api.test/v1/chat");
}

#[test]
fn the_url_uses_the_environment_base() {
    let (url, base) = url_of(
        &passthrough_config(Some("https://env.test/v1"), None),
        None,
        "chat",
        None,
    );
    assert_eq!(url.as_str(), "https://env.test/v1/chat");
    assert_eq!(base, "https://env.test/v1");
}

#[test]
fn the_url_uses_the_default_base() {
    let (url, _) = url_of(&passthrough_config(None, None), None, "chat", None);
    assert_eq!(url.as_str(), format!("{GIGACHAT_BASE_URL}/chat"));
}

#[test]
fn the_url_ignores_the_query_params() {
    let query = request_data(r#"{"api-version": "2024"}"#);
    let (url, _) = url_of(
        &passthrough_config(None, None),
        Some("https://api.test/v1"),
        "chat",
        Some(&query),
    );
    assert!(url.query().is_none());
}

#[test]
fn the_api_key_wins_over_the_environment_key() {
    let api_key = SecretString::from("api-key");
    let config = passthrough_config(None, Some("env-key"));
    assert_eq!(
        config
            .get_api_key(Some(&api_key))
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("api-key"))
    );
}

#[test]
fn the_environment_key_is_used_without_one() {
    let config = passthrough_config(None, Some("env-key"));
    assert_eq!(
        config
            .get_api_key(None)
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("env-key"))
    );
}

#[test]
fn an_empty_api_key_falls_back_to_the_environment_key() {
    let api_key = SecretString::from("");
    let config = passthrough_config(None, Some("env-key"));
    assert_eq!(
        config
            .get_api_key(Some(&api_key))
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("env-key"))
    );
}

#[test]
fn an_empty_environment_key_counts_as_unset() {
    let config = passthrough_config(None, Some(""));
    assert_eq!(
        config
            .get_api_key(None)
            .map(|key| key.expose_secret().to_owned()),
        None
    );
}

#[test]
fn no_key_is_none() {
    assert_eq!(
        passthrough_config(None, None)
            .get_api_key(None)
            .map(|key| key.expose_secret().to_owned()),
        None
    );
}

#[tokio::test]
async fn the_token_and_json_headers_are_added() {
    let bearer = ScriptedBearer::issuing("token-1");
    let headers = validate_environment(Headers::new(), None, &bearer)
        .await
        .unwrap();
    assert_eq!(
        headers,
        serde_json::from_str::<Headers>(
            r#"{"Accept": "application/json", "Authorization": "Bearer token-1", "Content-Type": "application/json"}"#
        )
        .unwrap()
    );
}

#[tokio::test]
async fn caller_headers_are_kept_and_exact_keys_are_overridden() {
    let bearer = ScriptedBearer::issuing("token-1");
    let caller: Headers = serde_json::from_str(
        r#"{"Authorization": "Bearer caller", "Accept": "text/plain", "accept": "text/html", "X-Trace": "abc"}"#,
    )
    .unwrap();
    let headers = validate_environment(caller, None, &bearer).await.unwrap();
    assert_eq!(
        headers,
        serde_json::from_str::<Headers>(
            r#"{"Accept": "application/json", "Authorization": "Bearer token-1", "Content-Type": "application/json", "X-Trace": "abc", "accept": "text/html"}"#
        )
        .unwrap()
    );
}

#[tokio::test]
async fn a_failed_token_keeps_its_status() {
    let bearer = ScriptedBearer::failing(401);
    let error = validate_environment(Headers::new(), None, &bearer)
        .await
        .unwrap_err();
    match error {
        GigachatPassthroughError::Token(source) => assert_eq!(source.status_code, 401),
        other @ (GigachatPassthroughError::Passthrough(_)
        | GigachatPassthroughError::Provider(_)) => {
            panic!("expected a token error, got {other:?}")
        }
    }
}

#[tokio::test]
async fn a_set_credential_is_passed_to_the_bearer() {
    let bearer = ScriptedBearer::issuing("token-1");
    let credential = SecretString::from("client:secret");
    validate_environment(Headers::new(), Some(&credential), &bearer)
        .await
        .unwrap();
    assert_eq!(bearer.seen(), vec![Some(String::from("client:secret"))]);
}

#[tokio::test]
async fn an_empty_credential_is_not_passed_to_the_bearer() {
    let bearer = ScriptedBearer::issuing("token-1");
    let credential = SecretString::from("");
    validate_environment(Headers::new(), Some(&credential), &bearer)
        .await
        .unwrap();
    assert_eq!(bearer.seen(), vec![None]);
}

#[test]
fn chat_logging_parses_a_completion_reply() {
    let logged = logged_value(
        &passthrough_config(None, None),
        "v1/chat/completions",
        "gigachat/GigaChat",
        r#"{"id":"chat-1","created":17,"model":"BodyModel","choices":[{"index":0,"message":{"role":"assistant","content":"hey"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
    );
    let Some(GigachatPassthroughLogged::Chat(reply)) = logged else {
        panic!("expected a chat reply");
    };
    assert_eq!(reply.id, "chat-1");
    assert_eq!(reply.created, 17);
    assert_eq!(reply.model, "gigachat/GigaChat");
    assert_eq!(reply.object, "chat.completion");
    let choice = reply.choices.first().expect("one choice");
    assert_eq!(choice.index, 0);
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content.as_deref(), Some("hey"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert_eq!(reply.usage.prompt_tokens, 3);
    assert_eq!(reply.usage.completion_tokens, 2);
    assert_eq!(reply.usage.total_tokens, 5);
}

#[test]
fn a_reply_without_created_uses_zero() {
    let logged = logged_value(
        &passthrough_config(None, None),
        "v1/chat/completions",
        "m",
        r#"{"id":"chat-1","model":"m","choices":[]}"#,
    );
    let Some(GigachatPassthroughLogged::Chat(reply)) = logged else {
        panic!("expected a chat reply");
    };
    assert_eq!(reply.created, 0);
}

#[test]
fn a_chat_reply_that_is_not_json_gives_none() {
    assert!(
        logged_value(
            &passthrough_config(None, None),
            "v1/chat/completions",
            "m",
            "{nope"
        )
        .is_none()
    );
}

#[test]
fn embedding_logging_parses_an_embedding_reply() {
    let logged = logged_value(
        &passthrough_config(None, None),
        "v1/embeddings",
        "gigachat/Embeddings",
        r#"{"object":"list","model":"Embeddings","data":[{"object":"embedding","embedding":[1],"index":0,"usage":{"prompt_tokens":4}},{"object":"embedding","embedding":[2],"index":1,"usage":{"prompt_tokens":6}}]}"#,
    );
    let Some(GigachatPassthroughLogged::Embedding(reply)) = logged else {
        panic!("expected an embedding reply");
    };
    assert_eq!(reply.object, "list");
    assert_eq!(reply.model, "Embeddings");
    assert_eq!(reply.data.len(), 2);
    assert_eq!(reply.usage.prompt_tokens, 10);
    assert_eq!(reply.usage.total_tokens, 10);
}

#[test]
fn an_embedding_reply_without_a_model_gives_none() {
    assert!(logged_value(&passthrough_config(None, None), "v1/embeddings", "m", "{}").is_none());
}

#[test]
fn an_endpoint_with_both_names_is_a_completion() {
    let logged = logged_value(
        &passthrough_config(None, None),
        "v1/embeddings/completions",
        "m",
        r#"{"id":"chat-1","model":"m","choices":[]}"#,
    );
    assert!(matches!(logged, Some(GigachatPassthroughLogged::Chat(_))));
}

#[test]
fn another_endpoint_gives_none() {
    assert!(logged_value(&passthrough_config(None, None), "v1/models", "m", "{}").is_none());
}

#[test]
fn collected_chunks_keep_their_order() {
    let chunks = collected(
        &passthrough_config(None, None),
        &[
            r#"data: {"choices":[{"index":0,"delta":{"content":"one"}}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{"content":"two"}}]}"#,
        ],
    );
    assert_eq!(chunk_texts(chunks), vec!["one", "two"]);
}

#[test]
fn an_empty_chunk_is_skipped() {
    let chunks = collected(
        &passthrough_config(None, None),
        &[
            "",
            "   ",
            r#"data: {"choices":[{"index":0,"delta":{"content":"x"}}]}"#,
        ],
    );
    assert_eq!(chunk_texts(chunks), vec!["x"]);
}

#[test]
fn the_done_marker_is_skipped() {
    let chunks = collected(
        &passthrough_config(None, None),
        &[
            "[DONE]",
            " data: [DONE]",
            r#"{"choices":[{"index":0,"delta":{"content":"x"}}]}"#,
        ],
    );
    assert_eq!(chunk_texts(chunks), vec!["x"]);
}

#[test]
fn a_whitespace_padded_chunk_is_trimmed() {
    let chunks = collected(
        &passthrough_config(None, None),
        &[r#"  data: {"choices":[{"index":0,"delta":{"content":"x"}}]}  "#],
    );
    assert_eq!(chunk_texts(chunks), vec!["x"]);
}

#[test]
fn a_chunk_without_the_data_space_is_skipped() {
    assert!(
        collected(
            &passthrough_config(None, None),
            &[r#"data:{"choices":[{"index":0,"delta":{"content":"x"}}]}"#]
        )
        .is_none()
    );
}

#[test]
fn a_non_json_chunk_is_skipped() {
    assert!(
        collected(
            &passthrough_config(None, None),
            &["not json", "data: {oops}", r#"{"choices":[{"index":0}]}"#]
        )
        .is_some()
    );
    assert!(collected(&passthrough_config(None, None), &["not json"]).is_none());
}

#[test]
fn a_json_chunk_that_is_not_an_object_is_skipped() {
    assert!(collected(&passthrough_config(None, None), &["[1, 2]"]).is_none());
}

#[test]
fn no_chunks_gives_none() {
    assert!(collected(&passthrough_config(None, None), &[]).is_none());
}

#[test]
fn a_chunk_keeps_its_usage() {
    let chunks = collected(
        &passthrough_config(None, None),
        &[
            r#"data: {"choices":[{"index":0,"delta":{"content":"x"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
        ],
    );
    let Some(GigachatPassthroughLogged::Chunks(parsed)) = chunks else {
        panic!("expected collected chunks");
    };
    let chunk = parsed.first().expect("one chunk");
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, "stop");
    let usage = chunk.usage.as_ref().expect("a usage");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 5);
}

#[test]
fn a_usage_only_chunk_has_no_usage() {
    let chunks = collected(
        &passthrough_config(None, None),
        &[r#"data: {"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#],
    );
    let Some(GigachatPassthroughLogged::Chunks(parsed)) = chunks else {
        panic!("expected collected chunks");
    };
    let chunk = parsed.first().expect("one chunk");
    assert!(chunk.usage.is_none());
    assert!(!chunk.is_finished);
    assert_eq!(chunk.text, "");
}

#[test]
fn a_chunk_the_parser_cannot_decode_is_skipped() {
    assert!(
        collected(&passthrough_config(None, None), &[r#"{"choices":"wrong"}"#]).is_none(),
        "a chunk with a wrong field type is not a logged chunk"
    );
}
