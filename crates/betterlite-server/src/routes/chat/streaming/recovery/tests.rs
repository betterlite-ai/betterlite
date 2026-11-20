#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::routes::chat::chat_routes_with_call_id;
use crate::routes::chat::streaming::ChatCalls;
use crate::server::chat_setup::chat_executor;

const CHAT: &str = r#"{"model":"g","messages":[{"role":"user","content":"hi"}]}"#;
const STREAM: &str = r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"stream":true}"#;
const WITH_TEMPERATURE: &str =
    r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"temperature":0.5}"#;
const ANSWER: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o-2024-08-06","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
// LiteLLM's _OUTPUT_TOKEN_LIMIT_ERROR_MARKER inside the provider's own sentence
const LIMIT_ERROR: &str = r#"{"error":{"message":"You could not finish the message because max_tokens or model output limit was reached. Please try again with higher max_tokens.","type":"invalid_request_error","param":"max_tokens","code":"invalid_request_error"}}"#;
const UNRELATED: &str = r#"{"error":{"message":"bad request","type":"invalid_request_error"}}"#;
const REJECTED_TEMPERATURE: &str = r#"{"error":{"message":"{\"detail\":[{\"loc\":[\"body\",\"temperature\"],\"msg\":\"extra inputs are not permitted\"}]}","type":"invalid_request_error","param":"temperature","code":"invalid_request_error"}}"#;
const DROP_PARAMS: &str = "litellm_settings: {drop_params: true}\n";
const RECOVERED_ID: &str = "chatcmpl-00000000-0000-0000-0000-000000000007";
const RECOVERED_CREATED: u64 = 1_700_000_000;

type Check = fn() -> Pin<Box<dyn Future<Output = ()>>>;

#[rstest]
#[case::buffered_length_reply(buffered_length_reply)]
#[case::stream_two_frames(stream_two_frames)]
#[case::unrelated_400_answers_400(unrelated_400_answers_400)]
#[case::drop_params_resend(drop_params_resend)]
#[tokio::test]
async fn output_limit_route_cases(#[case] check: Check) {
    check().await;
}

#[derive(Deserialize)]
struct Completion {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<CompletedChoice>,
    usage: CompletedUsage,
}

#[derive(Deserialize)]
struct CompletedChoice {
    message: CompletedMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct CompletedMessage {
    role: String,
    content: Option<String>,
}

#[derive(Deserialize)]
struct CompletedUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Deserialize)]
struct Frame {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<FrameChoice>,
}

#[derive(Deserialize)]
struct FrameChoice {
    delta: FrameDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct FrameDelta {
    role: Option<String>,
    content: Option<String>,
}

#[derive(Deserialize)]
struct Failure {
    error: FailureBody,
}

#[derive(Deserialize)]
struct FailureBody {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
}

fn fixed_clock() -> u64 {
    RECOVERED_CREATED
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

// the provider's answers in order, and the bodies it was sent, read back from a loopback listener
async fn serve(listener: TcpListener, replies: Vec<(u16, &'static str)>) -> Vec<String> {
    let mut bodies = Vec::new();
    for (status, reply) in replies {
        let Ok(accepted) = timeout(Duration::from_secs(2), listener.accept()).await else {
            break;
        };
        let (mut socket, _) = accepted.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(socket.read_u8().await.unwrap());
        }
        let length: usize = String::from_utf8(head)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0_u8; length];
        socket.read_exact(&mut body).await.unwrap();
        bodies.push(String::from_utf8(body).unwrap());
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\nx-request-id: up-1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }
    bodies
}

// one booted chat router whose deployment points at a loopback provider answering the given replies in order
async fn answered(
    sent: &'static str,
    config_tail: &str,
    replies: Vec<(u16, &'static str)>,
) -> (StatusCode, HeaderMap, Vec<u8>, Vec<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k, timeout: 10}}}}\n{config_tail}"
    );
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source)
    .unwrap();
    let mut executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    executor.clock = fixed_clock;
    executor.next_id = fixed_id;
    let calls = ChatCalls::new(&config, Arc::new(executor)).unwrap();
    let router = chat_routes_with_call_id(Arc::new(calls), false).unwrap();
    let request = Request::post("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(sent))
        .unwrap();
    let (bodies, answered) = tokio::join!(serve(listener, replies), router.oneshot(request));
    let response = answered.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, body, bodies)
}

fn buffered_length_reply() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (status, headers, body, bodies) = answered(CHAT, "", vec![(400, LIMIT_ERROR)]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers
                .get("llm_provider-x-request-id")
                .map(|value| value.to_str().unwrap()),
            Some("up-1")
        );
        let reply: Completion = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply.id, RECOVERED_ID);
        assert_eq!(reply.object, "chat.completion");
        assert_eq!(reply.created, RECOVERED_CREATED);
        assert_eq!(reply.model, "g");
        assert_eq!(reply.choices.len(), 1);
        assert_eq!(reply.choices[0].message.role, "assistant");
        assert_eq!(reply.choices[0].message.content.as_deref(), Some(""));
        assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("length"));
        assert!(reply.usage.prompt_tokens > 0);
        assert_eq!(reply.usage.completion_tokens, 0);
        assert_eq!(reply.usage.total_tokens, reply.usage.prompt_tokens);
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("\"model\":\"up\""), "{}", bodies[0]);
        assert!(bodies[0].contains("\"messages\""), "{}", bodies[0]);
    })
}

fn frame_json(frame: &str) -> Frame {
    serde_json::from_str(frame.strip_prefix("data: ").unwrap()).unwrap()
}

fn stream_two_frames() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (status, headers, body, bodies) = answered(STREAM, "", vec![(400, LIMIT_ERROR)]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers
                .get("content-type")
                .map(|value| value.to_str().unwrap()),
            Some("text/event-stream; charset=utf-8")
        );
        let text = String::from_utf8(body).unwrap();
        let frames: Vec<&str> = text
            .split("\n\n")
            .filter(|frame| !frame.is_empty())
            .collect();
        assert_eq!(frames.len(), 3, "{text}");
        let first = frame_json(frames[0]);
        assert_eq!(first.id, RECOVERED_ID);
        assert_eq!(first.object, "chat.completion.chunk");
        assert_eq!(first.created, RECOVERED_CREATED);
        assert_eq!(first.model, "g");
        assert_eq!(first.choices.len(), 1);
        assert_eq!(first.choices[0].delta.role.as_deref(), Some("assistant"));
        assert_eq!(first.choices[0].delta.content.as_deref(), Some(""));
        assert_eq!(first.choices[0].finish_reason, None);
        let second = frame_json(frames[1]);
        assert_eq!(second.id, first.id);
        assert_eq!(second.choices[0].delta.role, None);
        assert_eq!(second.choices[0].delta.content, None);
        assert_eq!(second.choices[0].finish_reason.as_deref(), Some("length"));
        assert_eq!(frames[2], "data: [DONE]");
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("\"stream\":true"), "{}", bodies[0]);
    })
}

fn unrelated_400_answers_400() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (status, _headers, body, bodies) = answered(CHAT, "", vec![(400, UNRELATED)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let failure: Failure = serde_json::from_slice(&body).unwrap();
        assert_eq!(failure.error.error_type, "invalid_request_error");
        assert!(
            failure.error.message.contains("bad request"),
            "{}",
            failure.error.message
        );
        assert_eq!(bodies.len(), 1);
    })
}

fn drop_params_resend() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (status, _headers, body, bodies) = answered(
            WITH_TEMPERATURE,
            DROP_PARAMS,
            vec![(422, REJECTED_TEMPERATURE), (200, ANSWER)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let reply: Completion = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply.model, "g");
        assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("\"temperature\":0.5"), "{}", bodies[0]);
        assert!(!bodies[1].contains("temperature"), "{}", bodies[1]);
        assert!(bodies[1].contains("\"messages\""), "{}", bodies[1]);
        assert!(bodies[1].contains("\"model\":\"up\""), "{}", bodies[1]);
    })
}
