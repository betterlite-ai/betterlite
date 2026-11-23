use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_config::loader::ConfigLoader;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::server::chat_setup::chat_executor;
use crate::state::ServerState;
use crate::state::shared::Shared;

use super::{Answered, answered};

const MODEL: &str = "claude-3-haiku-20240307";
const BUFFERED: &str = r#"{"model":"chat-model","prompt":"Once upon a time"}"#;
const STREAMED: &str = r#"{"model":"chat-model","prompt":"Once upon a time","stream":true}"#;
const USAGE_STREAMED: &str = r#"{"model":"chat-model","prompt":"Once upon a time","stream":true,"stream_options":{"include_usage":true}}"#;
const REPLY: &str = r#"{"id":"msg_bridge","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"text","text":"hello over anthropic"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;
const CACHED_REPLY: &str = r#"{"id":"msg_cached","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"text","text":"hello over anthropic"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":4,"output_tokens":2,"cache_creation_input_tokens":8,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":8,"ephemeral_1h_input_tokens":0}}}"#;
const TOOL_REPLY: &str = r#"{"id":"msg_tool","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"tool_use","id":"toolu_bridge","name":"lookup","input":{"a":1}}],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":5,"output_tokens":3}}"#;
const ERROR: &str =
    r#"{"type":"error","error":{"type":"invalid_request_error","message":"no such model"}}"#;
const EVENTS: [&str; 6] = [
    r#"{"type":"message_start","message":{"id":"msg_stream","role":"assistant","model":"claude-3-haiku-20240307","usage":{"input_tokens":10,"output_tokens":1}}}"#,
    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
    r#"{"type":"content_block_stop","index":0}"#,
    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}"#,
    r#"{"type":"message_stop"}"#,
];
const CACHED_EVENTS: [&str; 6] = [
    r#"{"type":"message_start","message":{"id":"msg_stream_usage","role":"assistant","model":"claude-3-haiku-20240307","usage":{"input_tokens":10,"output_tokens":1,"cache_creation_input_tokens":8,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":8,"ephemeral_1h_input_tokens":0}}}}"#,
    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
    r#"{"type":"content_block_stop","index":0}"#,
    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}"#,
    r#"{"type":"message_stop"}"#,
];

#[derive(Deserialize)]
struct TextReply {
    id: String,
    object: String,
    model: String,
    choices: Vec<TextChoice>,
    usage: TextUsage,
}

#[derive(Deserialize)]
struct TextChoice {
    text: Option<String>,
}

// convert_chat_to_text_completion keeps the chat usage as it was, so Anthropic's cache counters reach the text body
#[derive(Deserialize)]
struct TextUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    prompt_tokens_details: TextPromptDetails,
    cache_creation_input_tokens: i64,
    cache_read_input_tokens: i64,
}

#[derive(Deserialize)]
struct TextPromptDetails {
    cache_creation_token_details: Option<TextCacheCreation>,
}

#[derive(Deserialize)]
struct TextCacheCreation {
    ephemeral_5m_input_tokens: i64,
    ephemeral_1h_input_tokens: i64,
}

// the adapter's closing frame when the caller asks for usage; its details outrun the chat usage type
#[derive(Deserialize)]
struct UsageFrame {
    usage: Option<StreamedUsage>,
}

#[derive(Deserialize)]
struct StreamedUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    prompt_tokens_details: TextPromptDetails,
}

fn local_environment() -> BTreeMap<String, String> {
    BTreeMap::from([(
        String::from("LITELLM_LOCAL_ANTHROPIC_BETA_HEADERS"),
        String::from("true"),
    )])
}

fn source(base: &str) -> String {
    format!(
        "model_list:\n  - {{model_name: chat-model, litellm_params: {{model: anthropic/{MODEL}, api_base: '{base}', api_key: oracle-key, timeout: 30}}}}\n"
    )
}

fn local_state(base: &str) -> Arc<ServerState> {
    let environment = local_environment();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source(base))
    .expect("config");
    let executor =
        chat_executor(&config, &environment, StdRng::seed_from_u64(3)).expect("executor");
    let shared = Arc::new(Shared {
        bundled_catalog: Arc::clone(&executor.bundled_catalog),
        client: executor.client.clone(),
    });
    Arc::new(ServerState::new(&config, Arc::new(executor), shared))
}

async fn exchange(request: &str, answer: (u16, &str)) -> Answered {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let state = local_state(&format!("http://{address}"));
    answered(state, listener, "/v1/completions", request, answer).await
}

fn stream_events(events: &[&str]) -> String {
    events.iter().fold(String::new(), |mut body, event| {
        body.push_str("data: ");
        body.push_str(event);
        body.push_str("\n\n");
        body
    })
}

// the last frame the route sends with a usage object: the adapter's closing usage chunk
fn closing_usage(raw: &str) -> StreamedUsage {
    raw.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|frame| serde_json::from_str::<UsageFrame>(frame).ok())
        .filter_map(|frame| frame.usage)
        .next_back()
        .expect("the closing usage frame")
}

async fn buffered_text() {
    let answered = exchange(BUFFERED, (200, REPLY)).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(
        answered
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let reply: TextReply = serde_json::from_str(&answered.raw).expect("text answer");
    assert_reply(&reply);
    // the text conversion keeps the usage dump's null extras as LiteLLM writes them
    assert!(
        answered.raw.contains(r#""iterations":null"#),
        "{}",
        answered.raw
    );
    assert_prompt(&answered.sent);
}

fn assert_reply(reply: &TextReply) {
    assert_eq!(reply.id, "msg_bridge");
    assert_eq!(reply.object, "text_completion");
    assert_eq!(reply.model, "chat-model");
    assert_eq!(
        reply
            .choices
            .first()
            .and_then(|choice| choice.text.as_deref()),
        Some("hello over anthropic")
    );
    assert_eq!(reply.usage.prompt_tokens, 3);
    assert_eq!(reply.usage.completion_tokens, 2);
    assert_eq!(reply.usage.total_tokens, 5);
    assert_eq!(reply.usage.cache_creation_input_tokens, 0);
    assert_eq!(reply.usage.cache_read_input_tokens, 0);
    assert!(
        reply
            .usage
            .prompt_tokens_details
            .cache_creation_token_details
            .is_none()
    );
}

fn assert_prompt(sent: &str) {
    assert!(sent.starts_with("POST /v1/messages"), "{}", sent);
    let prompted =
        r#""messages":[{"role":"user","content":[{"type":"text","text":"Once upon a time"}]}]"#;
    assert!(sent.contains(prompted), "{}", sent);
    assert!(!sent.contains(r#""stream""#), "{}", sent);
}

async fn stream_text() {
    let events = stream_events(&EVENTS);
    let answered = exchange(STREAMED, (200, &events)).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(
        answered
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    assert!(
        answered.sent.contains(r#""stream":true"#),
        "{}",
        answered.sent
    );
    assert!(
        answered.raw.contains(r#""object":"text_completion""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.contains(r#""text":"hello""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.ends_with("data: [DONE]\n\n"),
        "{:?}",
        answered.raw
    );
}

// The closing usage frame copies Anthropic's usage as LiteLLM does, cache breakdown included, then data: [DONE]
async fn usage_stream_text() {
    let events = stream_events(&CACHED_EVENTS);
    let answered = exchange(USAGE_STREAMED, (200, &events)).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert!(
        answered.sent.contains(r#""stream":true"#),
        "{}",
        answered.sent
    );
    assert!(
        answered.raw.contains(r#""model":"chat-model""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.contains(r#""text":"hello""#),
        "{}",
        answered.raw
    );
    let usage = closing_usage(&answered.raw);
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 12);
    let details = usage
        .prompt_tokens_details
        .cache_creation_token_details
        .expect("the streamed cache breakdown");
    assert_eq!(details.ephemeral_5m_input_tokens, 8);
    assert_eq!(details.ephemeral_1h_input_tokens, 0);
    assert!(
        answered.raw.ends_with("data: [DONE]\n\n"),
        "{:?}",
        answered.raw
    );
}

async fn upstream_error() {
    let answered = exchange(BUFFERED, (400, ERROR)).await;
    assert_eq!(answered.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        answered
            .body
            .error
            .as_ref()
            .and_then(|error| error.code.as_deref()),
        Some("400")
    );
}

// The chat response cannot read Anthropic's cache breakdown, so a reply carrying one still answers its text body
async fn cache_details_text() {
    let answered = exchange(BUFFERED, (200, CACHED_REPLY)).await;
    assert_eq!(answered.status, StatusCode::OK);
    let reply: TextReply = serde_json::from_str(&answered.raw).expect("text answer");
    assert_eq!(reply.usage.cache_creation_input_tokens, 8);
    let details = reply
        .usage
        .prompt_tokens_details
        .cache_creation_token_details
        .expect("the reply's cache breakdown");
    assert_eq!(details.ephemeral_5m_input_tokens, 8);
    assert_eq!(details.ephemeral_1h_input_tokens, 0);
}

// LiteLLM's text conversion reads only the message content, so a tool call reply keeps its finish reason but not the call
async fn tool_call_text() {
    let answered = exchange(BUFFERED, (200, TOOL_REPLY)).await;
    assert_eq!(answered.status, StatusCode::OK);
    let reply: TextReply = serde_json::from_str(&answered.raw).expect("text answer");
    assert_eq!(reply.object, "text_completion");
    assert!(
        reply
            .choices
            .first()
            .is_some_and(|choice| choice.text.is_none())
    );
    assert!(
        answered.raw.contains(r#""finish_reason":"tool_calls""#),
        "{}",
        answered.raw
    );
    assert!(
        !answered.raw.contains(r#""tool_calls":["#),
        "{}",
        answered.raw
    );
}

#[derive(Clone, Copy)]
enum Scenario {
    BufferedText,
    StreamText,
    UsageStreamText,
    UpstreamError,
    CacheDetailsText,
    ToolCallText,
}

#[rstest]
#[case::buffered_text(Scenario::BufferedText)]
#[case::stream_text(Scenario::StreamText)]
#[case::upstream_error(Scenario::UpstreamError)]
#[case::cache_details_text(Scenario::CacheDetailsText)]
#[case::tool_call_text(Scenario::ToolCallText)]
#[case::usage_stream_text(Scenario::UsageStreamText)]
#[tokio::test]
async fn anthropic_bridge_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::BufferedText => buffered_text().await,
        Scenario::StreamText => stream_text().await,
        Scenario::UsageStreamText => usage_stream_text().await,
        Scenario::UpstreamError => upstream_error().await,
        Scenario::CacheDetailsText => cache_details_text().await,
        Scenario::ToolCallText => tool_call_text().await,
    }
}
