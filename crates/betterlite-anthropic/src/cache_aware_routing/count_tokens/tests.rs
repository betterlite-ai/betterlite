#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::anthropic_count_tokens::DEFAULT_BETA_VERSION;
use rstest::rstest;
use secrecy::SecretString;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::tests::support::BODY;
use crate::cache_aware_routing::tests::support::KEY;
use crate::cache_aware_routing::tests::support::MODEL;
use crate::cache_aware_routing::tests::support::StubTransport;
use crate::cache_aware_routing::tests::support::json;
use crate::cache_aware_routing::tests::support::settings;
use crate::count_tokens::handler::AnthropicCountTokensHandler;

fn handler(status: u16, payload: &str) -> AnthropicCountTokensHandler<StubTransport> {
    AnthropicCountTokensHandler {
        transport: StubTransport::replying(status, payload),
        request_timeout: Duration::from_secs(1),
        beta_version: DEFAULT_BETA_VERSION.to_owned(),
    }
}

async fn count(
    status: u16,
    payload: &str,
    api_base: &str,
    body: &str,
) -> (Option<i64>, AnthropicCountTokensHandler<StubTransport>) {
    let handler = handler(status, payload);
    let settings = settings(api_base);
    let key = SecretString::from(KEY.to_owned());
    let count =
        AnthropicCacheRouting::count_tokens(&handler, &settings, MODEL, &key, &json(body)).await;
    (count, handler)
}

#[tokio::test]
async fn count_tokens_sends_the_native_count_to_the_messages_endpoint() {
    let (count, handler) = count(
        200,
        r#"{"input_tokens":5}"#,
        "https://api.anthropic.com",
        BODY,
    )
    .await;
    assert_eq!(count, Some(5));
    let sent = handler.transport.request();
    assert_eq!(
        sent.url,
        "https://api.anthropic.com/v1/messages/count_tokens"
    );
    assert_eq!(sent.api_key.as_deref(), Some(KEY));
}

#[tokio::test]
async fn count_tokens_appends_the_count_path_to_a_messages_base() {
    let api_base = "https://api.anthropic.com/v1/messages";
    let (count, handler) = count(200, r#"{"input_tokens":9}"#, api_base, BODY).await;
    assert_eq!(count, Some(9));
    assert_eq!(
        handler.transport.request().url,
        "https://api.anthropic.com/v1/messages/count_tokens"
    );
}

#[tokio::test]
async fn count_tokens_uses_a_custom_base() {
    let (count, handler) = count(200, r#"{"input_tokens":3}"#, "https://proxy.test", BODY).await;
    assert_eq!(count, Some(3));
    assert_eq!(
        handler.transport.request().url,
        "https://proxy.test/v1/messages/count_tokens"
    );
}

#[rstest]
#[case::upstream_error(500, r#"{"type":"error"}"#)]
#[case::missing_count(200, "{}")]
#[tokio::test]
async fn count_tokens_turns_an_unusable_reply_into_none(
    #[case] status: u16,
    #[case] payload: &str,
) {
    let (count, _handler) = count(status, payload, "https://api.anthropic.com", BODY).await;
    assert_eq!(count, None);
}

#[tokio::test]
async fn count_tokens_keeps_a_zero_count() {
    let (count, _handler) = count(
        200,
        r#"{"input_tokens":0}"#,
        "https://api.anthropic.com",
        BODY,
    )
    .await;
    assert_eq!(count, Some(0));
}

#[tokio::test]
async fn count_tokens_sends_nothing_for_a_body_without_messages() {
    let (count, handler) = count(
        200,
        r#"{"input_tokens":5}"#,
        "https://api.anthropic.com",
        r#"{"model":"claude-x","max_tokens":16}"#,
    )
    .await;
    assert_eq!(count, None);
    assert!(handler.transport.requests().is_empty());
}
