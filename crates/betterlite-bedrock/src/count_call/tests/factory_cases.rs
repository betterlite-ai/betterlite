use crate::count_call::counter::TokenCounter as _;
use crate::count_call::factory::BedrockModelInfo;

use super::support::{FixtureCredentials, Reply, Script, call, environment, header, user_messages};

pub(crate) async fn counter_factory() {
    fresh_counters().await;
    environment_is_carried().await;
    selection_is_on_the_counter();
}

async fn fresh_counters() {
    let messages = user_messages();
    let credentials = FixtureCredentials::new("AKIDFACTORY", "factory-secret", None);
    let info = BedrockModelInfo::default();
    let first_transport = Script::replying(Reply::ok(r#"{"inputTokens": 11}"#));
    let second_transport = Script::replying(Reply::ok(r#"{"inputTokens": 12}"#));
    let first = info.get_token_counter(first_transport.clone(), credentials.clone());
    let second = info.get_token_counter(second_transport.clone(), credentials.clone());
    assert_eq!(credentials.calls(), 0);
    assert_eq!(
        first
            .count_tokens(&call(&messages, None))
            .await
            .unwrap()
            .total_tokens,
        11
    );
    assert_eq!(
        second
            .count_tokens(&call(&messages, None))
            .await
            .unwrap()
            .total_tokens,
        12
    );
    assert_eq!(first_transport.requests().len(), 1);
    assert_eq!(second_transport.requests().len(), 1);
    assert_eq!(credentials.calls(), 2);
}

async fn environment_is_carried() {
    let messages = user_messages();
    let info = BedrockModelInfo {
        environment: environment(&[("AWS_BEDROCK_RUNTIME_ENDPOINT", "https://factory.example")]),
    };
    let transport = Script::replying(Reply::ok(r#"{"inputTokens": 14}"#));
    let counter = info.get_token_counter(
        transport.clone(),
        FixtureCredentials::new("AKIDFACTORY", "factory-secret", None),
    );
    counter.count_tokens(&call(&messages, None)).await.unwrap();
    let sent = transport.request();
    assert_eq!(
        sent.url,
        "https://factory.example/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
    assert!(
        header(&sent, "authorization")
            .unwrap()
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDFACTORY/")
    );
}

fn selection_is_on_the_counter() {
    let counter = BedrockModelInfo::default().get_token_counter(
        Script::replying(Reply::ok("{}")),
        FixtureCredentials::new("AKIDFACTORY", "factory-secret", None),
    );
    assert!(counter.should_use_token_counting_api(Some("bedrock")));
    assert!(!counter.should_use_token_counting_api(Some("bedrock_converse")));
}
