#![cfg(test)]

use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;
use url::Url;

use super::{AiohttpOpenAi, DEFAULT_AIOHTTP_BASE, complete_url};
use crate::operation::{
    ChatCompletion, ChatOperation, EmbeddingOperation, OperationError, OperationRequest,
    OperationResult,
};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};

fn deployment(api_base: &str, api_key: Option<&str>) -> Deployment {
    Deployment {
        model: String::from("up"),
        api_base: api_base.to_owned(),
        endpoint: Url::parse("http://127.0.0.1:9/v1/chat/completions").unwrap(),
        api_key: api_key.map(SecretString::from),
        authorization: None,
        organization: Some(String::from("org-1")),
        timeout: Duration::from_secs(30),
        max_retries: None,
        drop_params: None,
        provider_affinity_header: None,
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn chat(stream: bool) -> OperationRequest {
    let operation = Box::new(ChatOperation {
        model: String::from("up"),
        request: ChatRequest::parse(br#"{"model":"g","messages":[]}"#).unwrap(),
        control: ControlOptions::default(),
    });
    if stream {
        OperationRequest::Stream(operation)
    } else {
        OperationRequest::Chat(operation)
    }
}

fn prepared(api_base: &str, api_key: Option<&str>, stream: bool) -> PreparedRequest {
    AiohttpOpenAi
        .prepare(&chat(stream), &deployment(api_base, api_key), &catalog())
        .unwrap()
}

fn buffered(status: u16, body: &str) -> Upstream {
    Upstream::Buffered(UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    })
}

const FULL_ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","system_fingerprint":"fp","service_tier":"auto","choices":[{"index":0,"message":{"role":"assistant","content":"hi","refusal":null},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;

fn bare_base_gets_the_chat_path() {
    assert_eq!(
        complete_url(DEFAULT_AIOHTTP_BASE),
        "https://api.openai.com/chat/completions"
    );
    assert_eq!(
        complete_url("https://host/v1/chat/completions"),
        "https://host/v1/chat/completions"
    );
    let PreparedRequest::Chat(chat) = prepared("https://host/v1", Some("k"), false) else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(chat.http.url.as_str(), "https://host/v1/chat/completions");
}

fn bearer_never_falls_back_to_the_environment() {
    let PreparedRequest::Chat(keyed) = prepared("https://host", Some("configured-key"), false)
    else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(
        keyed.http.headers.get("authorization").unwrap(),
        "Bearer configured-key"
    );
    assert!(keyed.http.headers.get("openai-organization").is_none());
    let PreparedRequest::Chat(keyless) = prepared("https://host", None, false) else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(
        keyless.http.headers.get("authorization").unwrap(),
        "Bearer None"
    );
}

fn response_copies_only_the_projected_fields() {
    let prepared = prepared("https://host", Some("k"), false);
    let decoded = AiohttpOpenAi
        .decode(&prepared, buffered(200, FULL_ANSWER))
        .unwrap();
    let OperationResult::Chat(ChatCompletion::AiohttpOpenAi(completion)) = decoded else {
        panic!("the aiohttp projection decodes to its own completion");
    };
    let written: JsonValue =
        serde_json::from_slice(&serde_json::to_vec(&*completion).unwrap()).unwrap();
    let wanted: JsonValue = serde_json::from_str(
        r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","system_fingerprint":"fp","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#,
    )
    .unwrap();
    assert_eq!(written, wanted);
}

fn provider_error_status_is_a_provider_error() {
    let prepared = prepared("https://host", Some("k"), false);
    let Err(OperationError::Provider { response }) =
        AiohttpOpenAi.decode(&prepared, buffered(500, "{}"))
    else {
        panic!("a failing status is a provider error");
    };
    assert_eq!(response.status, 500);
}

fn stream_request_keeps_the_stream_variant() {
    assert!(matches!(
        prepared("https://host", Some("k"), true),
        PreparedRequest::Stream(_)
    ));
}

fn embedding_is_unsupported_before_http() {
    let embedding = OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: String::from("up"),
        request: serde_json::from_str(r#"{"input":"hello"}"#).unwrap(),
    }));
    let refused = AiohttpOpenAi
        .prepare(
            &embedding,
            &deployment("https://host", Some("k")),
            &catalog(),
        )
        .unwrap_err();
    assert!(matches!(
        refused,
        OperationError::UnsupportedOperation { .. }
    ));
}

#[rstest]
#[case::bare_base(bare_base_gets_the_chat_path)]
#[case::bearer_without_fallback(bearer_never_falls_back_to_the_environment)]
#[case::projection_drops_usage(response_copies_only_the_projected_fields)]
#[case::provider_error(provider_error_status_is_a_provider_error)]
#[case::stream_variant(stream_request_keeps_the_stream_variant)]
#[case::embedding_unsupported(embedding_is_unsupported_before_http)]
fn provider_framework_tests_behavior(#[case] check: fn()) {
    check();
}
