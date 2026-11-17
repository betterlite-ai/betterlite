use betterlite_models::controls::json_value::JsonValue;
use secrecy::SecretString;

use super::support;
use crate::openai_dispatch::call::EmbeddingCall;
use crate::openai_dispatch::fixtures::FixtureAnswer;
use crate::operation::OperationResult;

pub async fn check() {
    let mut composition = support::composition();
    composition.env_api_key = Some(SecretString::from("sk-env"));
    composition.globals.organization = Some(String::from("org-global"));
    composition.provider.default_embedding_encoding_format = Some(String::from("base64"));
    let call = EmbeddingCall {
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
    };
    let answer = FixtureAnswer::new(200, support::EMBEDDING_REPLY.as_bytes().to_vec());
    let result = support::dispatch_embedding(&composition, call, &answer)
        .await
        .expect("the embedding dispatch");
    let sent = answer.requests();
    assert_eq!(sent.len(), 1);
    let sent = &sent[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.url, "https://explicit.test/v1/embeddings");
    assert_eq!(sent.header("authorization"), Some("Bearer sk-env"));
    assert_eq!(sent.header("accept"), Some("application/json"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    // B006 builds its client without an organization, so neither the explicit nor the global one is sent
    assert_eq!(sent.header("openai-organization"), None);
    let body = support::body_fields(&sent.body);
    assert_eq!(
        support::field(&body, "model"),
        &support::text_value("text-embedding-3-small")
    );
    assert_eq!(
        support::field(&body, "dimensions"),
        &support::number_value(8)
    );
    assert_eq!(support::field(&body, "user"), &support::text_value("u1"));
    // the none sentinel drops the field, and the mock fields never reach a provider
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

pub async fn check_configured_format() {
    let mut composition = support::composition();
    composition.globals.api_key = Some(SecretString::from("sk-global"));
    composition.provider.default_embedding_encoding_format = Some(String::from("base64"));
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
