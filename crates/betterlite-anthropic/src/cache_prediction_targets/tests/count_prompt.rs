use std::time::Duration;

use betterlite_models::decisions::Json;
use http::Method;
use rstest::rstest;

use super::support::{KEY, MODEL, Pending, Script, Slow, json, secret, settings};
use crate::cache_prediction_targets::counting::{
    PromptCount, PromptTokenCounter, TokenCounter as _, count_prompt_tokens,
};
use crate::count_tokens::handler::AnthropicCountTokensHandler;

const SUCCESS: &[u8] = br#"{"input_tokens":7}"#;
const BODY: &str = r#"{"model":"claude-x","messages":[{"role":"user","content":"hello"}],"max_tokens":5,"system":"be brief","tools":[{"name":"t","input_schema":{}}],"thinking":{"type":"enabled"},"tool_choice":null,"temperature":0}"#;
const EXPECTED: &str = r#"{"model":"claude-x","messages":[{"role":"user","content":"hello"}],"system":"be brief","tools":[{"name":"t","input_schema":{}}],"thinking":{"type":"enabled"},"tool_choice":null}"#;
const MINIMAL: &str = r#"{"messages":[{"role":"user","content":"hi"}]}"#;

fn handler(transport: Script) -> AnthropicCountTokensHandler<Script> {
    AnthropicCountTokensHandler {
        transport,
        request_timeout: Duration::from_secs(600),
        beta_version: "test-beta".to_owned(),
    }
}

async fn count_with(
    script: Script,
    body: &Json,
    api_base: Option<&str>,
) -> (Option<i64>, AnthropicCountTokensHandler<Script>) {
    let handler = handler(script);
    let settings = settings("https://api.anthropic.com", None);
    let key = secret(KEY);
    let call = PromptCount {
        model: MODEL,
        api_key: &key,
        body,
        api_base,
    };
    let count = count_prompt_tokens(&handler, &settings, &call).await;
    (count, handler)
}

async fn counted(body: &str) -> (Option<i64>, AnthropicCountTokensHandler<Script>) {
    let script = Script::replying(&[(200, SUCCESS)]);
    count_with(script, &json(body), None).await
}

#[tokio::test]
async fn count_prompt_tokens_posts_the_native_count_endpoint() {
    let (count, handler) = count_with(Script::replying(&[(200, SUCCESS)]), &json(BODY), None).await;
    assert_eq!(count, Some(7));
    let sent = handler.transport.request();
    assert_eq!(sent.method, Method::POST);
    assert_eq!(
        sent.url,
        "https://api.anthropic.com/v1/messages/count_tokens"
    );
    assert_eq!(sent.header("x-api-key"), Some(KEY));
    assert_eq!(sent.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(sent.deadline, Duration::from_secs(15));
    let body = serde_json::from_slice::<Json>(&sent.body).unwrap();
    assert_eq!(body, json(EXPECTED));
}

#[tokio::test]
async fn count_prompt_tokens_uses_the_given_api_base() {
    let (count, handler) = count_with(
        Script::replying(&[(200, SUCCESS)]),
        &json(MINIMAL),
        Some("https://configured.test"),
    )
    .await;
    assert_eq!(count, Some(7));
    assert_eq!(
        handler.transport.request().url,
        "https://configured.test/v1/messages/count_tokens"
    );
}

#[tokio::test]
async fn count_prompt_tokens_keeps_a_messages_url_unchanged() {
    let (_count, handler) = count_with(
        Script::replying(&[(200, SUCCESS)]),
        &json(MINIMAL),
        Some("https://configured.test/v1/messages"),
    )
    .await;
    assert_eq!(
        handler.transport.request().url,
        "https://configured.test/v1/messages/count_tokens"
    );
}

#[tokio::test]
async fn count_prompt_tokens_counts_through_the_token_counter() {
    let handler = AnthropicCountTokensHandler {
        transport: Script::replying(&[(200, SUCCESS)]),
        request_timeout: Duration::from_secs(600),
        beta_version: "test-beta".to_owned(),
    };
    let settings = settings("https://api.anthropic.com", None);
    let key = secret(KEY);
    let body = json(MINIMAL);
    let counter = PromptTokenCounter {
        handler: &handler,
        messages_url: &settings,
    };
    assert_eq!(counter.count(MODEL, &key, &body).await, Some(7));
    assert_eq!(
        handler.transport.request().url,
        "https://api.anthropic.com/v1/messages/count_tokens"
    );
}

#[tokio::test]
async fn count_prompt_tokens_accepts_a_system_block_list() {
    let body = r#"{"messages":[{"role":"user","content":"hi"}],"system":[{"type":"text","text":"be brief"}]}"#;
    let (count, handler) = counted(body).await;
    assert_eq!(count, Some(7));
    let sent = serde_json::from_slice::<Json>(&handler.transport.request().body).unwrap();
    assert_eq!(
        sent,
        json(
            r#"{"model":"claude-x","messages":[{"role":"user","content":"hi"}],"system":[{"type":"text","text":"be brief"}]}"#
        )
    );
}

#[tokio::test]
async fn count_prompt_tokens_omits_absent_options() {
    let (_count, handler) = counted(MINIMAL).await;
    let sent = serde_json::from_slice::<Json>(&handler.transport.request().body).unwrap();
    assert_eq!(
        sent,
        json(r#"{"model":"claude-x","messages":[{"role":"user","content":"hi"}]}"#)
    );
}

#[rstest]
#[case::not_an_object("null")]
#[case::missing_messages(r#"{"model":"claude-x"}"#)]
#[case::messages_null(r#"{"messages":null}"#)]
#[case::messages_not_a_list(r#"{"messages":"hi"}"#)]
#[case::message_not_a_mapping(r#"{"messages":["hi"]}"#)]
#[case::tools_not_a_list(r#"{"messages":[{"role":"user","content":"hi"}],"tools":"x"}"#)]
#[case::system_wrong_type(r#"{"messages":[{"role":"user","content":"hi"}],"system":5}"#)]
#[case::thinking_not_a_mapping(r#"{"messages":[{"role":"user","content":"hi"}],"thinking":[]}"#)]
#[tokio::test]
async fn count_prompt_tokens_rejects_an_invalid_body(#[case] body: &str) {
    let (count, handler) = counted(body).await;
    assert_eq!(count, None);
    assert!(handler.transport.requests().is_empty());
}

#[rstest]
#[case::upstream_error(400, b"bad")]
#[case::missing_input_tokens(200, b"{}")]
#[case::fractional_input_tokens(200, br#"{"input_tokens":1.5}"#)]
#[case::boolean_input_tokens(200, br#"{"input_tokens":true}"#)]
#[case::negative_input_tokens(200, br#"{"input_tokens":-1}"#)]
#[case::text_input_tokens(200, br#"{"input_tokens":"3"}"#)]
#[case::response_not_an_object(200, b"[1]")]
#[tokio::test]
async fn count_prompt_tokens_returns_none_on_a_provider_failure(
    #[case] status: u16,
    #[case] payload: &[u8],
) {
    let script = Script::replying(&[(status, payload)]);
    let (count, _handler) = count_with(script, &json(MINIMAL), None).await;
    assert_eq!(count, None);
}

#[tokio::test]
async fn count_prompt_tokens_returns_none_when_the_transport_fails() {
    let (count, _handler) = count_with(Script::failing(), &json(MINIMAL), None).await;
    assert_eq!(count, None);
}

#[tokio::test]
async fn count_prompt_tokens_accepts_a_zero_count() {
    let (count, _handler) = count_with(
        Script::replying(&[(200, br#"{"input_tokens":0}"#)]),
        &json(MINIMAL),
        None,
    )
    .await;
    assert_eq!(count, Some(0));
}

#[tokio::test(start_paused = true)]
async fn count_prompt_tokens_returns_none_when_the_count_times_out() {
    let handler = AnthropicCountTokensHandler {
        transport: Pending,
        request_timeout: Duration::from_secs(600),
        beta_version: "test-beta".to_owned(),
    };
    let settings = settings("https://api.anthropic.com", None);
    let key = secret(KEY);
    let body = json(MINIMAL);
    let call = PromptCount {
        model: MODEL,
        api_key: &key,
        body: &body,
        api_base: None,
    };
    assert_eq!(count_prompt_tokens(&handler, &settings, &call).await, None);
}

#[tokio::test(start_paused = true)]
async fn count_prompt_tokens_accepts_a_count_inside_the_timeout() {
    let handler = AnthropicCountTokensHandler {
        transport: Slow(Duration::from_secs(14)),
        request_timeout: Duration::from_secs(600),
        beta_version: "test-beta".to_owned(),
    };
    let settings = settings("https://api.anthropic.com", None);
    let key = secret(KEY);
    let body = json(MINIMAL);
    let call = PromptCount {
        model: MODEL,
        api_key: &key,
        body: &body,
        api_base: None,
    };
    assert_eq!(
        count_prompt_tokens(&handler, &settings, &call).await,
        Some(7)
    );
}
