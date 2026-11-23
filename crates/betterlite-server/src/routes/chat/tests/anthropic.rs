use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::Request;
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

use crate::routes::chat::chat_routes_with_call_id;
use crate::routes::chat::streaming::ChatCalls;
use crate::server::chat_setup::chat_executor;
use crate::state::anthropic_runtime;
use crate::state::azure_runtime;
use crate::state::shared::Shared;

use super::{Exchange, header};

const MODEL: &str = "claude-3-haiku-20240307";
const CHAT: &str = r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"temperature":0.2,"max_tokens":20}"#;
const STREAM_CHAT: &str =
    r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"stream":true}"#;
const REPLY: &str = r#"{"id":"msg_route","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"text","text":"hello over anthropic"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;
const ERROR: &str = r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#;
const TOOL_CHAT: &str =
    r#"{"model":"g","messages":[{"role":"user","content":"use a tool"}],"max_tokens":20}"#;
const TOOL_REPLY: &str = r#"{"id":"msg_tool","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"tool_use","id":"toolu_route","name":"lookup","input":{"a":1}}],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":5,"output_tokens":3}}"#;
const JSON_HEADERS: &str = "content-type: application/json\r\nrequest-id: req-route\r\nanthropic-ratelimit-requests-limit: 50\r\n";
const STREAM_HEADERS: &str = "content-type: text/event-stream\r\nrequest-id: req-stream\r\n";
const EVENTS: [&str; 6] = [
    r#"{"type":"message_start","message":{"id":"msg_stream","role":"assistant","model":"claude-3-haiku-20240307","usage":{"input_tokens":10,"output_tokens":1}}}"#,
    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
    r#"{"type":"content_block_stop","index":0}"#,
    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}"#,
    r#"{"type":"message_stop"}"#,
];

struct Answer<'body> {
    status: u16,
    headers: &'static str,
    body: &'body str,
}

#[derive(Deserialize)]
struct Reply {
    object: String,
    model: String,
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ErrorReply {
    error: ErrorFields,
}

#[derive(Deserialize)]
struct ErrorFields {
    code: String,
}

// the booted chat router over one config; the loaded Config drops here, not across the exchange's awaits
pub(super) fn calls(config: &Config, executor: ChatExecutor) -> ChatCalls {
    let shared = Shared {
        bundled_catalog: Arc::clone(&executor.bundled_catalog),
        client: executor.client.clone(),
    };
    let slot = anthropic_runtime::Slot::new(config, &shared);
    let azure = azure_runtime::Slot::new(config);
    ChatCalls::new(config, Arc::new(executor), &slot, &azure).expect("calls")
}

fn booted(config: &Config, environment: &BTreeMap<String, String>) -> Router {
    let executor = chat_executor(config, environment, StdRng::seed_from_u64(3)).expect("executor");
    chat_routes_with_call_id(Arc::new(calls(config, executor)), false).expect("routes")
}

fn source(base: &str, key: &str) -> String {
    format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: anthropic/{MODEL}, api_base: '{base}'{key}, timeout: 30}}}}\n"
    )
}

fn local_environment() -> BTreeMap<String, String> {
    BTreeMap::from([(
        String::from("LITELLM_LOCAL_ANTHROPIC_BETA_HEADERS"),
        String::from("true"),
    )])
}

fn stream_events() -> String {
    EVENTS.iter().fold(String::new(), |mut body, event| {
        body.push_str("data: ");
        body.push_str(event);
        body.push_str("\n\n");
        body
    })
}

fn reply() -> Answer<'static> {
    Answer {
        status: 200,
        headers: JSON_HEADERS,
        body: REPLY,
    }
}

async fn provider(listener: TcpListener, answer: Answer<'_>) -> String {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(1), listener.accept()).await else {
        return String::new();
    };
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let mut body = vec![0_u8; 16 * 1024];
    let read = stream.read(&mut body).await.unwrap();
    body.truncate(read);
    received.extend(body);
    let head = format!(
        "HTTP/1.1 {} X\r\n{}content-length: {}\r\n\r\n",
        answer.status,
        answer.headers,
        answer.body.len()
    );
    let _written = stream
        .write_all(format!("{head}{}", answer.body).as_bytes())
        .await;
    String::from_utf8(received).unwrap()
}

async fn exchange(
    key: &str,
    environment: &BTreeMap<String, String>,
    request: &str,
    answer: Answer<'_>,
) -> Exchange {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let source = source(&base, key);
    let router = {
        let config = ConfigLoader {
            lookup: environment.clone(),
        }
        .load(&source)
        .unwrap();
        booted(&config, environment)
    };
    let built = Request::post("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(request.to_owned()))
        .unwrap();
    let upstream = Box::pin(provider(listener, answer));
    let (sent, answered) = tokio::join!(upstream, Box::pin(router.oneshot(built)));
    let response = answered.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    Exchange {
        status,
        headers,
        body,
        sent,
        base,
    }
}

async fn buffered_reply() {
    let answered = exchange(", api_key: route-key", &local_environment(), CHAT, reply()).await;
    assert_eq!(answered.status, 200);
    assert_eq!(
        header(&answered.headers, "x-litellm-model-api-base"),
        Some(answered.base.as_str())
    );
    let text = String::from_utf8(answered.body).unwrap();
    // the proxy dumps with exclude_none, so a null system_fingerprint never reaches the client
    assert!(!text.contains("system_fingerprint"), "{text}");
    let body: Reply = serde_json::from_str(&text).unwrap();
    assert_eq!(body.object, "chat.completion");
    assert_eq!(body.model, "g");
    assert_eq!(
        body.choices[0].message.content.as_deref(),
        Some("hello over anthropic")
    );
    assert_eq!(
        header(&answered.headers, "llm_provider-request-id"),
        Some("req-route")
    );
    assert_eq!(
        header(
            &answered.headers,
            "llm_provider-anthropic-ratelimit-requests-limit"
        ),
        Some("50")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-model-group"),
        Some("g")
    );
    assert!(
        answered.sent.starts_with("POST /v1/messages"),
        "{}",
        answered.sent
    );
}

async fn stream_frames() {
    let events = stream_events();
    let answer = Answer {
        status: 200,
        headers: STREAM_HEADERS,
        body: &events,
    };
    let answered = exchange(
        ", api_key: route-key",
        &local_environment(),
        STREAM_CHAT,
        answer,
    )
    .await;
    assert_eq!(answered.status, 200);
    assert_eq!(
        header(&answered.headers, "content-type"),
        Some("text/event-stream; charset=utf-8")
    );
    let text = String::from_utf8(answered.body).unwrap();
    assert!(text.ends_with("data: [DONE]\n\n"), "{text}");
    assert!(text.matches("data: ").count() >= 2, "{text}");
    assert!(text.contains("hello"), "{text}");
}

async fn upstream_error() {
    let answer = Answer {
        status: 429,
        headers: JSON_HEADERS,
        body: ERROR,
    };
    let answered = exchange(", api_key: route-key", &local_environment(), CHAT, answer).await;
    assert_eq!(answered.status, 429);
    let body: ErrorReply = serde_json::from_slice(&answered.body).unwrap();
    assert_eq!(body.error.code, "429");
}

// LiteLLM's dump keeps a null content beside the tool calls, as OpenAI's message shape requires it
async fn tool_call_reply() {
    let answer = Answer {
        status: 200,
        headers: JSON_HEADERS,
        body: TOOL_REPLY,
    };
    let answered = exchange(
        ", api_key: route-key",
        &local_environment(),
        TOOL_CHAT,
        answer,
    )
    .await;
    assert_eq!(answered.status, 200);
    let text = String::from_utf8(answered.body).unwrap();
    assert!(text.contains("\"content\":null"), "{text}");
    assert!(text.contains("\"tool_calls\":[{"), "{text}");
}

async fn own_key_only() {
    let environment = BTreeMap::from([
        (
            String::from("LITELLM_LOCAL_ANTHROPIC_BETA_HEADERS"),
            String::from("true"),
        ),
        (
            String::from("OPENAI_API_KEY"),
            String::from("openai-env-key"),
        ),
        (
            String::from("OPENAI_BASE_URL"),
            String::from("http://127.0.0.1:9"),
        ),
        (
            String::from("ANTHROPIC_API_KEY"),
            String::from("anthropic-env-key"),
        ),
    ]);
    let answered = exchange("", &environment, CHAT, reply()).await;
    assert_eq!(answered.status, 200);
    let sent = answered.sent.to_lowercase();
    assert!(
        sent.contains("x-api-key: anthropic-env-key"),
        "{}",
        answered.sent
    );
    assert!(!sent.contains("openai-env-key"), "{}", answered.sent);
    assert!(
        answered.sent.starts_with("POST /v1/messages"),
        "{}",
        answered.sent
    );
}

#[derive(Clone, Copy)]
enum Scenario {
    BufferedReply,
    StreamFrames,
    UpstreamError,
    OwnKeyOnly,
    ToolCallReply,
}

#[rstest]
#[case::buffered_reply(Scenario::BufferedReply)]
#[case::stream_frames(Scenario::StreamFrames)]
#[case::upstream_error(Scenario::UpstreamError)]
#[case::own_key_only(Scenario::OwnKeyOnly)]
#[case::tool_call_reply(Scenario::ToolCallReply)]
#[tokio::test]
async fn anthropic_route_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::BufferedReply => buffered_reply().await,
        Scenario::StreamFrames => stream_frames().await,
        Scenario::UpstreamError => upstream_error().await,
        Scenario::OwnKeyOnly => own_key_only().await,
        Scenario::ToolCallReply => tool_call_reply().await,
    }
}
