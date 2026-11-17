use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use super::support;
use crate::openai_dispatch::call::{Composition, EmbeddingCall};
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::operation::OperationResult;

fn composition() -> Composition {
    let mut composition = support::composition();
    composition.env_api_key = Some(SecretString::from("sk-env"));
    composition.globals.organization = Some(String::from("org-global"));
    composition.provider.embedding_format = Some(String::from("base64"));
    composition
}

fn call() -> EmbeddingCall {
    EmbeddingCall {
        model: String::from("text-embedding-3-small"),
        request: support::embedding_request(
            r#"{"input":"hello","encoding_format":"float","dimensions":8,"user":"u1","mock_response":"mocked"}"#,
        ),
        encoding_format: Some(String::from("none")),
        credentials: support::credentials(
            Some("https://explicit.test/v1"),
            None,
            Some("org-explicit"),
        ),
        headers: None,
        timeout: Some(20.0),
    }
}

async fn dispatched() -> (FixtureAnswer, OperationResult) {
    let answer = FixtureAnswer::new(200, support::EMBEDDING_REPLY.as_bytes().to_vec());
    let result = support::dispatch_embedding(&composition(), call(), &answer)
        .await
        .expect("the embedding dispatch");
    (answer, result)
}

// The request carries the resolved base and environment key, and B006 sends no organization.
pub(super) async fn check_request() {
    let (answer, _result) = dispatched().await;
    let sent = answer.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, "https://explicit.test/v1/embeddings");
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-env"));
    assert_eq!(sent[0].header("accept"), Some("application/json"));
    assert_eq!(sent[0].header("content-type"), Some("application/json"));
    assert_eq!(sent[0].header("openai-organization"), None);
}

// The body keeps the caller's fields, drops the none sentinel and the mock fields, and the reply decodes.
pub(super) async fn check_body_and_reply() {
    let (answer, result) = dispatched().await;
    let body = support::body_fields(&answer.requests()[0].body);
    assert_eq!(
        support::field(&body, "model"),
        &support::text_value("text-embedding-3-small")
    );
    assert_eq!(
        support::field(&body, "dimensions"),
        &support::number_value(8)
    );
    assert_eq!(support::field(&body, "user"), &support::text_value("u1"));
    assert!(!body.contains_key("encoding_format"));
    assert!(!body.contains_key("mock_response"));
    let OperationResult::Embedding(reply) = result else {
        panic!("an embedding reply");
    };
    assert_eq!(reply.object, "list");
    assert_eq!(reply.model, "text-embedding-3-small");
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.usage.prompt_tokens, 2);
}

pub(super) async fn check_configured_format() {
    let mut composition = support::composition();
    composition.globals.api_key = Some(SecretString::from("sk-global"));
    composition.provider.embedding_format = Some(String::from("base64"));
    let call = EmbeddingCall {
        model: String::from("text-embedding-3-small"),
        request: support::embedding_request(r#"{"input":["a","b"]}"#),
        encoding_format: None,
        credentials: support::credentials(None, None, None),
        headers: None,
        timeout: None,
    };
    let answer = FixtureAnswer::new(200, support::EMBEDDING_REPLY.as_bytes().to_vec());
    let _result = support::dispatch_embedding(&composition, call, &answer)
        .await
        .expect("the embedding dispatch");
    let sent = answer.requests();
    assert_eq!(sent[0].url, "https://api.openai.com/v1/embeddings");
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-global"));
    // no organization is configured, so the request carries none
    assert_eq!(sent[0].header("openai-organization"), None);
    let body = support::body_fields(&sent[0].body);
    assert_eq!(
        support::field(&body, "encoding_format"),
        &support::text_value("base64")
    );
    let JsonValue::Array(input) = support::field(&body, "input") else {
        panic!("an input array");
    };
    assert_eq!(input.len(), 2);
}

// An empty explicit format falls through to the request's own value, which wins over the configured default.
pub(super) async fn check_format_falls_through() {
    let mut composition = support::composition();
    composition.provider.embedding_format = Some(String::from("float"));
    let call = EmbeddingCall {
        model: String::from("text-embedding-3-small"),
        request: support::embedding_request(r#"{"input":"hello","encoding_format":"base64"}"#),
        encoding_format: Some(String::new()),
        credentials: support::credentials(
            Some("https://explicit.test/v1"),
            Some("sk-explicit"),
            None,
        ),
        headers: None,
        timeout: None,
    };
    let answer = FixtureAnswer::new(200, support::EMBEDDING_REPLY.as_bytes().to_vec());
    let _result = support::dispatch_embedding(&composition, call, &answer)
        .await
        .expect("the embedding dispatch");
    let body = support::body_fields(&answer.requests()[0].body);
    assert_eq!(
        support::field(&body, "encoding_format"),
        &support::text_value("base64")
    );
}

// The caller's header map reaches the embedding request over the generated auth and content-type.
pub(super) async fn check_caller_headers() {
    let mut call = call();
    call.headers = Some(Headers::from([(
        String::from("x-caller"),
        String::from("1"),
    )]));
    let answer = FixtureAnswer::new(200, support::EMBEDDING_REPLY.as_bytes().to_vec());
    let _result = support::dispatch_embedding(&composition(), call, &answer)
        .await
        .expect("the embedding dispatch");
    let sent = answer.requests();
    assert_eq!(sent[0].header("x-caller"), Some("1"));
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-env"));
}

pub(super) async fn check_all() {
    check_request().await;
    check_body_and_reply().await;
    check_configured_format().await;
    check_format_falls_through().await;
    check_caller_headers().await;
}
