#![cfg(test)]

mod support;
use crate::anthropic_dispatch::base::Surface;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;

#[rstest::rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
#[tokio::test]
async fn native_messages_uses_preparation_and_transport(#[case] asynchronous: bool) {
    let observed = support::run(
        Surface::Messages,
        asynchronous,
        true,
        ChatConfigTag::Anthropic,
    )
    .await;
    assert_eq!(observed.posts, 1);
    if asynchronous {
        assert!(observed.logged.contains("chatcmpl-native"));
    }
    assert_eq!(observed.content.as_deref(), Some("native answer"));
    assert_eq!(observed.body["model"], "claude-3-haiku-20240307");
    assert_eq!(observed.body["max_tokens"], 17);
    assert_eq!(observed.body["tools"][0]["name"], "bad_name");
    assert_eq!(observed.tool_name.as_deref(), Some("bad/name"));
    assert_eq!(observed.body["messages"][0]["content"][0]["text"], "hi");
    assert_eq!(observed.key, "native-key");
}

#[tokio::test]
async fn native_messages_rejects_headers_before_preparation() {
    let observed = support::run(Surface::Messages, false, false, ChatConfigTag::Anthropic).await;
    assert_eq!(observed.posts, 0);
    assert_eq!(observed.status, Some(401));
}

#[tokio::test]
async fn native_legacy_uses_http_and_response_bridge() {
    let observed = support::run(Surface::Legacy, false, true, ChatConfigTag::Anthropic).await;
    assert_eq!(observed.posts, 1);
    assert_eq!(observed.content.as_deref(), Some("legacy answer"));
    assert_eq!(observed.body["model"], "claude-2");
    assert_eq!(observed.body["prompt"], "\n\nHuman: hi\n\nAssistant: ");
    assert_eq!(observed.key, "native-key");
}

#[tokio::test]
async fn native_completion_requires_anthropic_config() {
    let observed = support::run(Surface::Messages, false, true, ChatConfigTag::JsonProvider).await;
    assert_eq!(observed.posts, 0);
    assert_eq!(observed.status, None);
    assert_eq!(observed.content, None);
}

#[test]
fn native_config_filters_prepared_headers_and_body() {
    use betterlite_anthropic::chat_call::config::ChatConfig as _;
    use betterlite_anthropic::headers::AnthropicHeaders;
    use betterlite_anthropic::request::PreparedRequest;
    use std::sync::Mutex;
    let matrix = serde_json::from_str(r#"{"anthropic":{"keep":"keep"}}"#).unwrap();
    let prepared = PreparedRequest {
        body: indexmap::IndexMap::new(),
        headers: AnthropicHeaders {
            values: indexmap::IndexMap::from([(
                "anthropic-beta".to_owned(),
                "keep,drop".to_owned(),
            )]),
        },
        tool_name_reverse: std::collections::BTreeMap::new(),
    };
    let config = super::config::Config {
        validated: Mutex::new(None),
        prepared: Some(prepared),
        matrix: &matrix,
    };
    let mut body = serde_json::from_str(r#"{"anthropic_beta":["keep","drop"]}"#).unwrap();
    let mut headers = indexmap::IndexMap::new();
    config
        .filter_request_betas("anthropic", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["anthropic-beta"], "keep");
    assert_eq!(
        serde_json::to_value(body).unwrap(),
        serde_json::json!({"anthropic_beta":["keep"]})
    );
}

#[test]
fn native_counter_uses_supplied_encoding() {
    use betterlite_anthropic::usage::ReasoningTokenCounter as _;
    use betterlite_tokenizer::engine::{
        Engine,
        assets::bundled,
        tiktoken::{Encoding, TiktokenEngine},
    };
    let encoding = Encoding::Cl100kBase;
    let engine = TiktokenEngine::from_bpe_file(encoding, &bundled(encoding).unwrap()).unwrap();
    let counter = super::Counter(Engine::Tiktoken(std::sync::Arc::new(engine)));
    assert_eq!(counter.count_reasoning_tokens("hello world").unwrap(), 2);
    assert_eq!(counter.count_reasoning_tokens("").unwrap(), 0);
}
