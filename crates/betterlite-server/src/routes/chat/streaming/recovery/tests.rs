#![cfg(test)]

use std::collections::BTreeMap;
use std::str::from_utf8;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_openai::chat::execution::ChatExecutor;
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
use crate::state::anthropic_runtime;
use crate::state::shared::Shared;

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

#[derive(Clone, Copy)]
enum Expectation {
    BufferedLength,
    StreamFrames,
    ProviderError,
    Resent,
}

#[rstest]
#[case::buffered_length_reply(CHAT, "", &[(400, LIMIT_ERROR)], Expectation::BufferedLength)]
#[case::stream_two_frames(STREAM, "", &[(400, LIMIT_ERROR)], Expectation::StreamFrames)]
#[case::unrelated_400_answers_400(CHAT, "", &[(400, UNRELATED)], Expectation::ProviderError)]
#[case::drop_params_resend(
    WITH_TEMPERATURE,
    DROP_PARAMS,
    &[(422, REJECTED_TEMPERATURE), (200, ANSWER)],
    Expectation::Resent
)]
#[tokio::test]
async fn output_limit_route_cases(
    #[case] sent: &'static str,
    #[case] config_tail: &'static str,
    #[case] replies: &'static [(u16, &'static str)],
    #[case] expected: Expectation,
) {
    let answer = answered(sent, config_tail, replies.to_vec()).await;
    match expected {
        Expectation::BufferedLength => buffered_reply(&answer),
        Expectation::StreamFrames => stream_frames(&answer),
        Expectation::ProviderError => provider_failure(&answer),
        Expectation::Resent => resent_after_rejection(&answer),
    }
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
    #[serde(rename = "completion_tokens")]
    completion: u64,
    #[serde(rename = "total_tokens")]
    total: u64,
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

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
    bodies: Vec<String>,
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

fn chat_calls(config: &Config, executor: ChatExecutor) -> ChatCalls {
    let shared = Shared {
        bundled_catalog: Arc::clone(&executor.bundled_catalog),
        client: executor.client.clone(),
    };
    let slot = anthropic_runtime::Slot::new(config, &shared);
    ChatCalls::new(config, Arc::new(executor), &slot).unwrap()
}

// one booted chat router whose deployment points at a loopback provider answering the given replies in order
async fn answered(
    sent: &'static str,
    config_tail: &str,
    replies: Vec<(u16, &'static str)>,
) -> Answer {
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
    let calls = chat_calls(&config, executor);
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
    Answer {
        status,
        headers,
        body,
        bodies,
    }
}

fn buffered_reply(answer: &Answer) {
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer
            .headers
            .get("llm_provider-x-request-id")
            .map(|value| value.to_str().unwrap()),
        Some("up-1")
    );
    assert_eq!(answer.bodies.len(), 1);
    assert!(
        answer.bodies[0].contains("\"model\":\"up\""),
        "{}",
        answer.bodies[0]
    );
    assert!(
        answer.bodies[0].contains("\"messages\""),
        "{}",
        answer.bodies[0]
    );
    let reply: Completion = serde_json::from_slice(&answer.body).unwrap();
    recorded_completion(&reply);
}

// the completion the wrapper builds for the eaten 400: empty text, the length reason, the counted prompt
fn recorded_completion(reply: &Completion) {
    assert_eq!(reply.id, RECOVERED_ID);
    assert_eq!(reply.object, "chat.completion");
    assert_eq!(reply.created, RECOVERED_CREATED);
    assert_eq!(reply.model, "g");
    assert_eq!(reply.choices.len(), 1);
    assert_eq!(reply.choices[0].message.role, "assistant");
    assert_eq!(reply.choices[0].message.content.as_deref(), Some(""));
    assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("length"));
    assert!(reply.usage.prompt_tokens > 0);
    assert_eq!(reply.usage.completion, 0);
    assert_eq!(reply.usage.total, reply.usage.prompt_tokens);
}

fn frame_json(frame: &str) -> Frame {
    serde_json::from_str(frame.strip_prefix("data: ").unwrap()).unwrap()
}

fn stream_frames(answer: &Answer) {
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer
            .headers
            .get("content-type")
            .map(|value| value.to_str().unwrap()),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(
        answer
            .headers
            .get("llm_provider-x-request-id")
            .map(|value| value.to_str().unwrap()),
        Some("up-1")
    );
    assert_eq!(answer.bodies.len(), 1);
    assert!(
        answer.bodies[0].contains("\"stream\":true"),
        "{}",
        answer.bodies[0]
    );
    let text = from_utf8(&answer.body).unwrap();
    let frames: Vec<&str> = text
        .split("\n\n")
        .filter(|frame| !frame.is_empty())
        .collect();
    assert_eq!(frames.len(), 3, "{text}");
    opening_frame(&frame_json(frames[0]));
    closing_frame(&frame_json(frames[1]));
    assert_eq!(frames[2], "data: [DONE]");
}

// the wrapper's first chunk: the assistant role with no content and no finish reason yet
fn opening_frame(first: &Frame) {
    assert_eq!(first.id, RECOVERED_ID);
    assert_eq!(first.object, "chat.completion.chunk");
    assert_eq!(first.created, RECOVERED_CREATED);
    assert_eq!(first.model, "g");
    assert_eq!(first.choices.len(), 1);
    assert_eq!(first.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(first.choices[0].delta.content.as_deref(), Some(""));
    assert_eq!(first.choices[0].finish_reason, None);
}

// the closing chunk: the same id, an empty delta and the held length reason
fn closing_frame(second: &Frame) {
    assert_eq!(second.id, RECOVERED_ID);
    assert_eq!(second.choices[0].delta.role, None);
    assert_eq!(second.choices[0].delta.content, None);
    assert_eq!(second.choices[0].finish_reason.as_deref(), Some("length"));
}

fn provider_failure(answer: &Answer) {
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let failure: Failure = serde_json::from_slice(&answer.body).unwrap();
    assert_eq!(failure.error.error_type, "invalid_request_error");
    // betterlite writes its own message for a provider error instead of copying the provider body
    assert_eq!(
        failure.error.message,
        "the provider answered with status 400"
    );
    assert_eq!(answer.bodies.len(), 1);
}

fn resent_after_rejection(answer: &Answer) {
    assert_eq!(answer.status, StatusCode::OK);
    let reply: Completion = serde_json::from_slice(&answer.body).unwrap();
    assert_eq!(reply.model, "g");
    assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(answer.bodies.len(), 2);
    assert!(
        answer.bodies[0].contains("\"temperature\":0.5"),
        "{}",
        answer.bodies[0]
    );
    assert!(
        !answer.bodies[1].contains("temperature"),
        "{}",
        answer.bodies[1]
    );
    assert!(
        answer.bodies[1].contains("\"messages\""),
        "{}",
        answer.bodies[1]
    );
    assert!(
        answer.bodies[1].contains("\"model\":\"up\""),
        "{}",
        answer.bodies[1]
    );
}
