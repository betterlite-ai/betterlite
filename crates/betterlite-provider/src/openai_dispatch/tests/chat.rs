use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::SecretString;

use super::support;
use crate::openai_dispatch::call::{ChatCall, Composition};
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::operation::{ChatCompletion, OperationResult};

fn call() -> ChatCall {
    ChatCall {
        model: String::from("gpt-4o"),
        request: support::chat_request(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"temperature":0.0}"#,
        ),
        credentials: support::credentials(
            Some("https://explicit.test/v1"),
            Some("sk-explicit"),
            None,
        ),
        headers: Some(BTreeMap::from([(
            String::from("x-header"),
            String::from("h"),
        )])),
        extra_headers: Some(BTreeMap::from([
            (String::from("x-header"), String::from("e")),
            (String::from("x-other"), String::from("1")),
        ])),
        timeout: Some(30.0),
        metadata: None,
    }
}

fn composition() -> Composition {
    let mut composition = support::composition();
    composition.globals = LiteLlmGlobals {
        api_base: Some(String::from("https://global.test/v1")),
        api_key: Some(SecretString::from("sk-global")),
        openai_key: None,
        organization: Some(String::from("org-global")),
    };
    composition.chat_defaults = support::chat_defaults(r#"{"temperature":0.5,"max_tokens":7}"#);
    composition
}

async fn dispatched() -> (FixtureAnswer, FixtureLogger, OperationResult) {
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_chat(&composition(), call(), &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    (answer, logger, result)
}

// The explicit identity names the call, the configured organization is sent beside it, and the SDK sends JSON.
pub(super) async fn check_request() {
    let (answer, _logger, _result) = dispatched().await;
    let sent = answer.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, "https://explicit.test/v1/chat/completions");
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-explicit"));
    assert_eq!(sent[0].header("openai-organization"), Some("org-global"));
    // completion() merges the caller's map with its extra headers
    assert_eq!(sent[0].header("accept"), Some("application/json"));
    assert_eq!(sent[0].header("content-type"), Some("application/json"));
    assert_eq!(sent[0].header("x-header"), Some("e"));
    assert_eq!(sent[0].header("x-other"), Some("1"));
}

// The body keeps the explicit zero over the configured default and fills the absent key, and the reply decodes.
pub(super) async fn check_body_and_reply() {
    let (answer, logger, result) = dispatched().await;
    let body = support::body_fields(&answer.requests()[0].body);
    assert_eq!(
        support::field(&body, "model"),
        &support::text_value("gpt-4o")
    );
    assert_eq!(
        support::field(&body, "temperature"),
        &JsonValue::Number(serde_json::Number::from_f64(0.0).expect("a number"))
    );
    assert_eq!(
        support::field(&body, "max_tokens"),
        &support::number_value(7)
    );
    let OperationResult::Chat(ChatCompletion::OpenAi(reply)) = result else {
        panic!("a chat completion");
    };
    assert_eq!(reply.id, "chatcmpl-1");
    assert_eq!(reply.object, "chat.completion");
    assert_eq!(reply.model, "gpt-4o-2024-08-06");
    assert_eq!(reply.choices.len(), 1);
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("hi"));
    assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(reply.usage.total_tokens, 2);
    // a successful non-streamed chat call has no post_call of main.py's
    assert!(logger.calls.is_empty());
}

pub(super) async fn check_all() {
    check_request().await;
    check_body_and_reply().await;
}
