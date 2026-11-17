use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::SecretString;

use super::support;
use crate::openai_dispatch::call::ChatCall;
use crate::openai_dispatch::fixtures::{FixtureAnswer, FixtureLogger};
use crate::operation::{ChatCompletion, OperationResult};

pub async fn check() {
    let mut composition = support::composition();
    composition.globals = LiteLlmGlobals {
        api_base: Some(String::from("https://global.test/v1")),
        api_key: Some(SecretString::from("sk-global")),
        openai_key: None,
        organization: Some(String::from("org-global")),
    };
    composition.chat_defaults = support::chat_defaults(r#"{"temperature":0.5,"max_tokens":7}"#);
    let call = ChatCall {
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
    };
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_chat(&composition, call, &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    let sent = answer.requests();
    assert_eq!(sent.len(), 1);
    let sent = &sent[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.url, "https://explicit.test/v1/chat/completions");
    assert_eq!(sent.header("authorization"), Some("Bearer sk-explicit"));
    assert_eq!(sent.header("accept"), Some("application/json"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(sent.header("openai-organization"), Some("org-global"));
    assert_eq!(sent.header("x-header"), Some("e"));
    assert_eq!(sent.header("x-other"), Some("1"));
    let body = support::body_fields(&sent.body);
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
