#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use betterlite_config::loader::ConfigLoader;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tower::ServiceExt as _;

use crate::routes::chat::chat_routes_with_call_id;
use crate::server::chat_setup::chat_executor;

mod defaults;
mod embedding_mapping;
mod errors;
mod mapping;
mod stream;

const OWN_KEY: &str = ", api_key: own-key";
const PLAIN: &str = r#"{"model":"g","messages":[]}"#;

const ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;

#[derive(Deserialize)]
struct Reply {
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    total_tokens: u64,
}

async fn upstream(listener: TcpListener, reply: (u16, &str)) -> String {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_millis(500), listener.accept()).await
    else {
        return String::new();
    };
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let mut body = vec![0_u8; 4096];
    let read = stream.read(&mut body).await.unwrap();
    let (code, answer) = reply;
    let head = format!(
        "HTTP/1.1 {code} Reply\r\nx-request-id: up-1\r\ncontent-length: {}\r\n\r\n{answer}",
        answer.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    body.truncate(read);
    received.extend(body);
    String::from_utf8(received).unwrap().to_lowercase()
}

async fn call(
    model: &str,
    sent_body: &'static str,
    key: &str,
    reply: (u16, &str),
) -> (u16, Vec<u8>, String, Option<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: {model}, api_base: '{base}'{key}}}, model_info: {{id: dep-1}}}}\n"
    );
    let environment = BTreeMap::from([
        ("OPENAI_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "OPENAI_BASE_URL".to_owned(),
            "http://127.0.0.1:1".to_owned(),
        ),
        ("OPENAI_ORGANIZATION".to_owned(), "org-9".to_owned()),
    ]);
    let loader = ConfigLoader {
        lookup: environment.clone(),
    };
    let config = loader.load(&source).unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    let request = Request::post("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(sent_body))
        .unwrap();
    let router = chat_routes_with_call_id(Arc::new(executor), false).unwrap();
    let (sent, answered) = tokio::join!(upstream(listener, reply), router.oneshot(request));
    let response = answered.unwrap();
    let status = response.status().as_u16();
    let forwarded = response
        .headers()
        .get("llm_provider-x-request-id")
        .map(|value| value.to_str().unwrap().to_owned());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, body, sent, forwarded)
}

#[rstest]
#[case::openai_keeps_usage_and_environment_organization("openai/up", PLAIN, 200, Some((3, true)))]
#[case::aiohttp_zeroes_usage_and_environment_organization("aiohttp_openai/up", PLAIN, 200, Some((0, false)))]
#[case::unavailable_adapter_fails_before_http("azure/up", PLAIN, 501, None)]
#[case::a_valid_chunk_size_reaches_the_provider("openai/up", r#"{"model":"g","messages":[],"stream_chunk_size":4}"#, 200, Some((3, true)))]
#[case::a_zero_chunk_size_fails_before_http(
    "openai/up",
    r#"{"model":"g","messages":[],"stream_chunk_size":0}"#,
    400,
    None
)]
#[tokio::test]
async fn provider_framework_tests_behavior(
    #[case] model: &str,
    #[case] sent_body: &'static str,
    #[case] status: u16,
    #[case] reached: Option<(u64, bool)>,
) {
    let (answered, body, sent, forwarded) = call(model, sent_body, OWN_KEY, (200, ANSWER)).await;
    assert_eq!(answered, status);
    let Some((total_tokens, sends_organization)) = reached else {
        assert_eq!(sent, "");
        return;
    };
    let reply: Reply = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        reply.usage.map(|usage| usage.total_tokens),
        Some(total_tokens)
    );
    assert_eq!(forwarded.as_deref(), Some("up-1"));
    assert!(sent.contains("authorization: bearer own-key"));
    assert_eq!(
        sent.contains("openai-organization: org-9"),
        sends_organization
    );
}
