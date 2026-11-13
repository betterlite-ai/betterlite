#![cfg(test)]

use betterlite_config::types::LiteLlmParams;
use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::count_tokens::counter::{AnthropicTokenCounter, CountTokensCall};
use crate::count_tokens::headers::get_required_headers;

use super::{
    MODEL, REQUEST_MODEL, Script, beta_handler, deployment, handler, header, secret, user_messages,
};

fn call<'call>(
    messages: &'call Json,
    deployment: Option<&'call LiteLlmParams>,
) -> CountTokensCall<'call> {
    CountTokensCall {
        model_to_use: MODEL,
        messages: Some(messages),
        contents: None,
        deployment,
        request_model: REQUEST_MODEL,
        tools: None,
        system: None,
    }
}

fn counter(
    beta: &str,
    script: Script,
    environment: Option<SecretString>,
) -> AnthropicTokenCounter<Script> {
    AnthropicTokenCounter {
        handler: beta_handler(script, beta),
        environment_api_key: environment,
    }
}

#[tokio::test]
async fn count_tokens_credential_identity_isolation() {
    separate_credentials().await;
    separate_environment_keys().await;
}

async fn separate_credentials() {
    let messages = user_messages();
    let alice = deployment(Some("sk-alice-identity"));
    let bob = deployment(Some("sk-bob-identity"));
    let alice_counter = counter(
        "alice-beta",
        Script::replying(200, b"{\"input_tokens\":1}"),
        None,
    );
    let bob_counter = counter(
        "bob-beta",
        Script::replying(200, b"{\"input_tokens\":2}"),
        None,
    );
    let alice_result = alice_counter
        .count_tokens(&call(&messages, Some(&alice)))
        .await
        .unwrap();
    let bob_result = bob_counter
        .count_tokens(&call(&messages, Some(&bob)))
        .await
        .unwrap();
    assert_eq!(alice_result.total_tokens, 1);
    assert_eq!(bob_result.total_tokens, 2);
    assert_separate_identities(&alice_counter, &bob_counter);
}

fn assert_separate_identities(
    alice_counter: &AnthropicTokenCounter<Script>,
    bob_counter: &AnthropicTokenCounter<Script>,
) {
    let alice_sent = alice_counter.handler.transport.request();
    let bob_sent = bob_counter.handler.transport.request();
    assert_eq!(header(&alice_sent, "x-api-key"), Some("sk-alice-identity"));
    assert_eq!(header(&bob_sent, "x-api-key"), Some("sk-bob-identity"));
    assert_eq!(header(&alice_sent, "anthropic-beta"), Some("alice-beta"));
    assert_eq!(header(&bob_sent, "anthropic-beta"), Some("bob-beta"));
    assert!(
        !alice_sent
            .headers
            .iter()
            .any(|(_, value)| value.contains("sk-bob-identity") || value.contains("bob-beta"))
    );
    assert!(
        !bob_sent
            .headers
            .iter()
            .any(|(_, value)| value.contains("sk-alice-identity") || value.contains("alice-beta"))
    );
}

async fn separate_environment_keys() {
    let messages = user_messages();
    let carol_counter = counter(
        "carol-beta",
        Script::replying(200, b"{\"input_tokens\":3}"),
        Some(secret("sk-carol-environment")),
    );
    let carol_result = carol_counter
        .count_tokens(&call(&messages, None))
        .await
        .unwrap();
    assert_eq!(carol_result.total_tokens, 3);
    let carol_sent = carol_counter.handler.transport.request();
    assert_eq!(
        header(&carol_sent, "x-api-key"),
        Some("sk-carol-environment")
    );
    assert_eq!(header(&carol_sent, "anthropic-beta"), Some("carol-beta"));
    assert!(!carol_sent.headers.iter().any(|(_, value)| {
        value.contains("sk-alice-identity") || value.contains("sk-bob-environment")
    }));
}

#[tokio::test]
async fn count_tokens_secret_safe_failure() {
    debug_output_is_safe();
    failure_output_is_safe().await;
    header_errors_are_safe();
}

fn debug_output_is_safe() {
    let messages = user_messages();
    let params = deployment(Some("sk-fixture-secret"));
    let fixture_counter = counter(
        "fixture-beta",
        Script::failing(),
        Some(secret("sk-fixture-environment-secret")),
    );
    let failing_call = call(&messages, Some(&params));
    assert!(!format!("{fixture_counter:?}").contains("sk-fixture-secret"));
    assert!(!format!("{fixture_counter:?}").contains("sk-fixture-environment-secret"));
    assert!(!format!("{:?}", fixture_counter.handler).contains("sk-fixture-secret"));
    assert!(!format!("{failing_call:?}").contains("sk-fixture-secret"));
}

async fn failure_output_is_safe() {
    let messages = user_messages();
    let params = deployment(Some("sk-fixture-secret"));
    let fixture_counter = AnthropicTokenCounter {
        handler: handler(Script::failing()),
        environment_api_key: Some(secret("sk-fixture-environment-secret")),
    };
    let failing_call = call(&messages, Some(&params));
    let outcome = fixture_counter.count_tokens(&failing_call).await.unwrap();
    assert!(outcome.error);
    let message = outcome.error_message.as_deref().unwrap();
    assert!(!message.contains("sk-fixture-secret"));
    assert!(!format!("{outcome:?}").contains("sk-fixture-secret"));
}

fn header_errors_are_safe() {
    let key = secret("sk-fixture-secret");
    let failure = get_required_headers(&key, "bad\nvalue")
        .unwrap_err()
        .to_string();
    assert!(!failure.contains("sk-fixture-secret"));
    let headers = get_required_headers(&key, "fixture-beta").unwrap();
    assert!(!format!("{headers:?}").contains("sk-fixture-secret"));
    assert!(headers["x-api-key"].is_sensitive());
    assert!(!headers["anthropic-version"].is_sensitive());
    assert!(!headers["content-type"].is_sensitive());
    let oauth = secret("sk-ant-oat-fixture-token");
    let oauth_headers = get_required_headers(&oauth, "fixture-beta").unwrap();
    assert!(!format!("{oauth_headers:?}").contains("sk-ant-oat-fixture-token"));
    assert!(oauth_headers["authorization"].is_sensitive());
    let empty_counter = AnthropicTokenCounter {
        handler: handler(Script::replying(200, b"{\"input_tokens\":1}")),
        environment_api_key: None,
    };
    assert!(!format!("{empty_counter:?}").contains("sk-fixture"));
}
