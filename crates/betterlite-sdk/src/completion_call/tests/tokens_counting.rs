use betterlite_models::decisions::Json;

use super::USER;
use super::support::{
    self, FakeLocalCounter, FakeProviderCounter, dig, json, secret, token_response,
};
use crate::completion_call::tokens::{LOCAL_TOKENIZER, TokenCountRequest, count_tokens};

fn token_messages() -> Vec<Json> {
    vec![json(USER)]
}

#[tokio::test]
async fn tokens_uses_the_provider_counter_when_it_answers() {
    let metadata = support::FakeMetadata::resolving("gpt-4o-2024");
    let mut answer = token_response(11, "gpt-4o", "gpt-4o-provider");
    answer.tokenizer_type = String::from("provider_tokenizer");
    let counter = FakeProviderCounter::answering(answer);
    let local = FakeLocalCounter::new(99);
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: None,
        api_base: None,
    };
    let response = count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 11);
    assert_eq!(response.tokenizer_type, "provider_tokenizer");
    assert_eq!(counter.seen().len(), 1);
    assert!(local.seen().is_empty());
}

#[tokio::test]
async fn tokens_provider_error_falls_back_to_the_local_count() {
    let metadata = support::FakeMetadata::resolving("gpt-4o-2024");
    let mut answer = token_response(11, "gpt-4o", "gpt-4o-provider");
    answer.error = true;
    let counter = FakeProviderCounter::answering(answer);
    let local = FakeLocalCounter::new(7);
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: None,
        api_base: None,
    };
    let response = count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 7);
    assert_eq!(response.tokenizer_type, LOCAL_TOKENIZER);
    assert_eq!(response.request_model, "gpt-4o");
    assert_eq!(response.model_used, "gpt-4o-2024");
    assert_eq!(local.seen().len(), 1);
}

#[tokio::test]
async fn tokens_skips_a_counter_that_does_not_serve_the_provider() {
    let metadata = support::FakeMetadata::default();
    let counter = FakeProviderCounter::unavailable();
    let local = FakeLocalCounter::new(3);
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: None,
        api_base: None,
    };
    let response = count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 3);
    assert!(counter.seen().is_empty());
    assert_eq!(local.seen().len(), 1);
}

#[tokio::test]
async fn tokens_system_message_is_prepended_only_with_messages() {
    let metadata = support::FakeMetadata::default();
    let local = FakeLocalCounter::new(5);
    let messages = token_messages();
    let with_messages = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: Some("rules"),
        api_key: None,
        api_base: None,
    };
    count_tokens(&with_messages, &metadata, None, &local)
        .await
        .unwrap();
    let seen = local.seen();
    assert_eq!(seen[0].len(), 2);
    assert_eq!(
        dig(&seen[0][0], &["role"]),
        Some(&Json::Text(String::from("system")))
    );
    let empty: [Json; 0] = [];
    let without_messages = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&empty),
        tools: None,
        system: Some("rules"),
        api_key: None,
        api_base: None,
    };
    count_tokens(&without_messages, &metadata, None, &local)
        .await
        .unwrap();
    assert!(local.seen()[1].is_empty());
}

#[tokio::test]
async fn tokens_explicit_key_and_base_win_over_the_resolution() {
    let metadata =
        support::FakeMetadata::resolving_credentials("https://resolved.test/v1", "resolved-key");
    let counter = FakeProviderCounter::answering(token_response(4, "gpt-4o", "m"));
    let local = FakeLocalCounter::new(0);
    let key = secret("explicit-key");
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: Some(&key),
        api_base: Some("https://explicit.test/v1"),
    };
    count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    let seen = counter.seen();
    assert_eq!(
        seen[0].api_base.as_deref(),
        Some("https://explicit.test/v1")
    );
    assert_eq!(seen[0].key.as_deref(), Some("explicit-key"));
}

#[tokio::test]
async fn tokens_absent_provider_answer_falls_back_to_the_local_count() {
    let metadata = support::FakeMetadata::default();
    let counter = FakeProviderCounter::silent();
    let local = FakeLocalCounter::new(9);
    let messages = token_messages();
    let token_request = TokenCountRequest {
        model: "gpt-4o",
        messages: Some(&messages),
        tools: None,
        system: None,
        api_key: None,
        api_base: None,
    };
    let response = count_tokens(&token_request, &metadata, Some(&counter), &local)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 9);
    assert_eq!(response.tokenizer_type, LOCAL_TOKENIZER);
}

#[test]
fn tokens_local_answer_carries_litellms_shape() {
    let response = token_response(12, "gpt-4o", "gpt-4o-2024");
    assert_eq!(response.total_tokens, 12);
    assert_eq!(response.tokenizer_type, "local_tokenizer");
    assert!(!response.error);
}
