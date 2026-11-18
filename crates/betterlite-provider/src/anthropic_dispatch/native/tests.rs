#![cfg(test)]

mod support;
use crate::anthropic_dispatch::base::Surface;
use betterlite_anthropic::chat_call::CallStyle;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::sync::Arc;

#[rstest::rstest]
#[case::sync(CallStyle::Sync)]
#[case::asynchronous(CallStyle::Async)]
#[test]
fn native_messages_uses_preparation_and_transport(#[case] style: CallStyle) {
    let observed = support::run(
        Surface::Messages,
        style,
        true,
        ChatConfigTag::Anthropic,
        "{}",
    );
    assert_eq!(observed.posts, 1);
    if style == CallStyle::Async {
        assert!(observed.logged.contains("chatcmpl-native"));
    }
    assert_eq!(observed.content.as_deref(), Some("native answer"));
    assert_eq!(observed.body.model, "claude-3-haiku-20240307");
    assert_eq!(observed.body.max_tokens, Some(17));
    assert_eq!(observed.body.tools[0].name, "bad_name");
    assert_eq!(observed.tool_name.as_deref(), Some("bad/name"));
    assert_eq!(observed.body.messages[0].content[0].text, "hi");
    assert_eq!(observed.key, "native-key");
}

#[test]
fn native_messages_rejects_headers_before_preparation() {
    let observed = support::run(
        Surface::Messages,
        CallStyle::Sync,
        false,
        ChatConfigTag::Anthropic,
        "{}",
    );
    assert_eq!(observed.posts, 0);
    assert_eq!(observed.status, Some(401));
}

#[test]
fn native_legacy_uses_http_and_response_bridge() {
    let observed = support::run(
        Surface::Legacy,
        CallStyle::Sync,
        true,
        ChatConfigTag::Anthropic,
        "{}",
    );
    assert_eq!(observed.posts, 1);
    assert_eq!(observed.content.as_deref(), Some("legacy answer"));
    assert_eq!(observed.body.model, "claude-2");
    assert_eq!(
        observed.body.prompt.as_deref(),
        Some("\n\nHuman: hi\n\nAssistant: ")
    );
    assert_eq!(observed.key, "native-key");
}

#[test]
fn native_completion_requires_anthropic_config() {
    let observed = support::run(
        Surface::Messages,
        CallStyle::Sync,
        true,
        ChatConfigTag::JsonProvider,
        "{}",
    );
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
        body: IndexMap::new(),
        headers: AnthropicHeaders {
            values: IndexMap::from([("anthropic-beta".to_owned(), "keep,drop".to_owned())]),
        },
        tool_name_reverse: BTreeMap::new(),
    };
    let config = super::config::Config {
        validated: Mutex::new(None),
        prepared: Some(prepared),
        matrix: &matrix,
    };
    let mut body = serde_json::from_str(r#"{"anthropic_beta":["keep","drop"]}"#).unwrap();
    let mut headers = IndexMap::new();
    config
        .filter_request_betas("anthropic", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["anthropic-beta"], "keep");
    assert_eq!(
        body,
        IndexMap::from([(
            "anthropic_beta".to_owned(),
            Json::List(vec![Json::Text("keep".to_owned())])
        )])
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
    let counter = super::Counter(Engine::Tiktoken(Arc::new(engine)));
    assert_eq!(counter.count_reasoning_tokens("hello world").unwrap(), 2);
    assert_eq!(counter.count_reasoning_tokens("").unwrap(), 0);
}

#[rstest::rstest]
#[case::absent(r#"{"max_tokens_to_sample":5}"#, 5)]
#[case::null(r#"{"max_tokens_to_sample":5,"extra_body":null}"#, 5)]
#[case::empty(r#"{"max_tokens_to_sample":5,"extra_body":{}}"#, 5)]
#[case::override_limit(
    r#"{"max_tokens_to_sample":5,"extra_body":{"max_tokens_to_sample":17,"top_k":3}}"#,
    17
)]
#[test]
fn native_legacy_merges_extra_body(#[case] options: &str, #[case] expected: i64) {
    let observed = support::run(
        Surface::Legacy,
        CallStyle::Sync,
        true,
        ChatConfigTag::Anthropic,
        options,
    );
    assert_eq!(observed.posts, 1);
    assert_eq!(observed.body.max_tokens_to_sample, Some(expected));
    assert!(!observed.body.extra.contains_key("extra_body"));
    if expected == 17 {
        assert_eq!(observed.body.top_k, Some(3));
    }
}

#[test]
fn native_preparation_error_retains_its_variant_and_status() {
    use betterlite_anthropic::request::{error::RequestError, tool_list::ToolListError};
    let observed = support::run(
        Surface::Messages,
        CallStyle::Sync,
        true,
        ChatConfigTag::Anthropic,
        r#"{"tools":[true]}"#,
    );
    assert_eq!(observed.posts, 0);
    let error = observed.preparation_error.unwrap();
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        *error,
        RequestError::ToolList(ToolListError::NotObject { index: 0 })
    ));
}

#[test]
fn native_legacy_stream_overrides_extra_body() {
    let observed = support::run(
        Surface::Legacy,
        CallStyle::Sync,
        true,
        ChatConfigTag::Anthropic,
        r#"{"stream":true,"extra_body":{"stream":false}}"#,
    );
    assert_eq!(observed.posts, 1);
    assert_eq!(observed.body.stream, Some(true));
    assert!(!observed.body.extra.contains_key("extra_body"));
    assert_eq!(
        observed.content.as_deref(),
        Some("data: {\"completion\":\"legacy answer\",\"stop_reason\":\"stop_sequence\"}\n\n")
    );
}

#[test]
fn native_messages_rejects_nonlist_tools_before_preparation() {
    let observed = support::run(
        Surface::Messages,
        CallStyle::Sync,
        true,
        ChatConfigTag::Anthropic,
        r#"{"tools":true}"#,
    );
    assert_eq!(observed.posts, 0);
    assert_eq!(observed.status, Some(400));
    assert!(observed.preparation_error.is_none());
}
