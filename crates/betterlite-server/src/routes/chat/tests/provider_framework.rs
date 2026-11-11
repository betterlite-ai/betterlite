use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use betterlite_config::loader::ConfigLoader;
use betterlite_models::controls::json_value::JsonValue;
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

const FULL_ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;

#[derive(Deserialize)]
struct Answer {
    usage: Option<JsonValue>,
}

async fn upstream(listener: TcpListener) -> String {
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
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{FULL_ANSWER}",
        FULL_ANSWER.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    body.truncate(read);
    received.extend(body);
    String::from_utf8(received).unwrap().to_lowercase()
}

async fn call(model: &str, key: &str) -> (u16, Vec<u8>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: {model}, api_base: '{base}', api_key: {key}}}, model_info: {{id: dep-1}}}}\n"
    );
    let environment = BTreeMap::from([
        ("OPENAI_API_KEY".to_owned(), "env-key".to_owned()),
        ("OPENAI_ORGANIZATION".to_owned(), "org-9".to_owned()),
    ]);
    let loader = ConfigLoader {
        lookup: environment.clone(),
    };
    let config = loader.load(&source).unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    let request = Request::post("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"model":"g","messages":[]}"#))
        .unwrap();
    let router = chat_routes_with_call_id(Arc::new(executor), false).unwrap();
    let (sent, answered) = tokio::join!(upstream(listener), router.oneshot(request));
    let response = answered.unwrap();
    let status = response.status().as_u16();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, body, sent)
}

#[rstest]
#[case::openai_keeps_usage_and_environment_organization("openai/up", 200, Some((true, true)))]
#[case::aiohttp_drops_usage_and_environment_organization("aiohttp_openai/up", 200, Some((false, false)))]
#[case::unavailable_adapter_fails_before_http("azure/up", 501, None)]
#[tokio::test]
async fn provider_framework_tests_behavior(
    #[case] model: &str,
    #[case] status: u16,
    #[case] reached: Option<(bool, bool)>,
) {
    let (answered, body, sent) = call(model, "own-key").await;
    assert_eq!(answered, status);
    let Some((has_usage, sends_organization)) = reached else {
        assert_eq!(sent, "");
        return;
    };
    let reply: Answer = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply.usage.is_some(), has_usage);
    assert!(sent.contains("authorization: bearer own-key"));
    assert_eq!(
        sent.contains("openai-organization: org-9"),
        sends_organization
    );
}
