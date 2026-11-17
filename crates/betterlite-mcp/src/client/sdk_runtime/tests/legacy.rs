use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::stream::{self, StreamExt as _};
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinSet;

use crate::client::sessions::error::SessionError;
use crate::client::sessions::stream::StreamFailureKind;
use crate::client::sessions::transport::TransportType;
use crate::client::tools::calling::{CallToolOptions, call_tool};
use crate::client::tools::listing::list_tools;
use crate::client::tools::result::{ContentBlock, ToolCallResult};
use crate::tools::schemas::McpTool;
use crate::tools::tool_calls::CallRequest;

use super::support::{self, RecordingHost};

const INSTRUCTIONS: &str = "the legacy server's instructions";
const ENDPOINT_EVENT: &str = "event: endpoint\ndata: /messages\n\n";
// A data line the MCP SDK's own SSE reader cannot parse, which its message handler reports as a ValueError
const BAD_EVENT: &str = "data: not-json\n\n";
const RAISE: CallToolOptions = CallToolOptions {
    raise_on_error: true,
    allow_input_required: false,
};

#[derive(Deserialize)]
struct WireRequest {
    #[serde(default)]
    id: Option<Box<RawValue>>,
    method: String,
    #[serde(default)]
    params: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct Cursored {
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

// The MCP SDK's SSE transport answers requests on the stream the client opened, so the server pushes frames through this
#[derive(Default)]
struct LegacyServer {
    outbox: Mutex<Option<UnboundedSender<String>>>,
}

impl LegacyServer {
    fn stream(&self) -> Body {
        let (sender, receiver) = unbounded_channel::<String>();
        let mut outbox = self.outbox.lock().unwrap_or_else(PoisonError::into_inner);
        let replaced = outbox.replace(sender);
        drop(replaced);
        drop(outbox);
        let opening = stream::once(async { Ok::<_, Infallible>(ENDPOINT_EVENT.to_owned()) });
        let rest = stream::unfold(receiver, |mut receiver| async move {
            let chunk = receiver.recv().await?;
            Some((Ok::<_, Infallible>(chunk), receiver))
        });
        Body::from_stream(opening.chain(rest))
    }

    fn push(&self, frame: String) {
        let outbox = self.outbox.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(sender) = outbox.as_ref() else {
            return;
        };
        drop(sender.send(frame));
    }
}

async fn open_events(State(server): State<Arc<LegacyServer>>) -> Response {
    let mut response = Response::new(server.stream());
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn receive_message(State(server): State<Arc<LegacyServer>>, body: Bytes) -> Response {
    let Ok(request) = serde_json::from_slice::<WireRequest>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(id) = request.id.as_deref() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let id = id.get().to_owned();
    let answer = match request.method.as_str() {
        "initialize" => Some(initialize_reply(&id)),
        "tools/list" => Some(list_reply(&id, request.params.as_deref())),
        "tools/call" => call_reply(&id, request.params.as_deref()),
        other => Some(failure_reply(&id, &format!("unknown method {other}"))),
    };
    let frame = match answer {
        Some(answer) => format!("data: {answer}\n\n"),
        None => BAD_EVENT.to_owned(),
    };
    server.push(frame);
    StatusCode::ACCEPTED.into_response()
}

fn initialize_reply(id: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}},"prompts":{{}},"resources":{{}}}},"serverInfo":{{"name":"legacy-test","version":"1.0"}},"instructions":"{INSTRUCTIONS}"}}}}"#
    )
}

const FIRST_PAGE: &str = r#"{"tools":[{"name":"alpha","description":"the first tool","inputSchema":{"type":"object"}},{"name":"beta","description":"the second tool","inputSchema":{"type":"object"}}],"nextCursor":"page-2"}"#;
const SECOND_PAGE: &str = r#"{"tools":[{"name":"gamma","description":"the third tool","inputSchema":{"type":"object"}}]}"#;
fn list_reply(id: &str, params: Option<&RawValue>) -> String {
    let page = match cursor(params).as_deref() {
        Some("page-2") => SECOND_PAGE,
        _ => FIRST_PAGE,
    };
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{page}}}"#)
}

fn cursor(params: Option<&RawValue>) -> Option<String> {
    let raw = params?;
    serde_json::from_str::<Cursored>(raw.get()).ok()?.cursor
}

fn call_reply(id: &str, params: Option<&RawValue>) -> Option<String> {
    match tool_name(params).as_deref() {
        Some("echo") => Some(echo_reply(id)),
        Some("stream-fail") => None,
        Some(other) => Some(failure_reply(id, &format!("unknown tool {other}"))),
        None => Some(failure_reply(id, "missing tool name")),
    }
}

fn tool_name(params: Option<&RawValue>) -> Option<String> {
    let raw = params?;
    serde_json::from_str::<Named>(raw.get())
        .ok()
        .map(|call| call.name)
}

fn echo_reply(id: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"type":"text","text":"echoed"}}]}}}}"#
    )
}

fn failure_reply(id: &str, message: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32602,"message":"{message}"}}}}"#)
}

struct LegacySseServer {
    url: String,
    _tasks: JoinSet<Result<(), io::Error>>,
}

async fn serve_legacy() -> LegacySseServer {
    let state = Arc::new(LegacyServer::default());
    let router = Router::new()
        .route("/sse", get(open_events))
        .route("/messages", post(receive_message))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { axum::serve(listener, router).await });
    LegacySseServer {
        url: format!("http://{address}/sse"),
        _tasks: tasks,
    }
}

fn call(name: &str) -> CallRequest {
    let name = name.to_owned();
    let arguments = None;
    CallRequest { name, arguments }
}

fn tool_names(tools: Vec<McpTool>) -> Vec<String> {
    tools.into_iter().map(|tool| tool.name).collect()
}

#[tokio::test]
async fn a_legacy_sse_session_lists_every_tool_page_and_calls_a_tool() {
    let server = serve_legacy().await;
    let mut client = support::client_for(
        support::runtime(),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let tools = list_tools(&mut client, support::CALL_TIMEOUT, true)
        .await
        .unwrap();
    assert_eq!(tool_names(tools), ["alpha", "beta", "gamma"]);
    assert_eq!(
        client.last_initialize_instructions.as_deref(),
        Some(INSTRUCTIONS)
    );
    let result = call_tool(
        &mut client,
        &call("echo"),
        None::<&RecordingHost>,
        CallToolOptions::default(),
    )
    .await
    .unwrap();
    let ToolCallResult::Complete(complete) = result else {
        panic!("expected a complete tool result");
    };
    match complete.content.as_slice() {
        [ContentBlock::Text(text)] => assert_eq!(text.text, "echoed"),
        other => panic!("unexpected content blocks: {other:?}"),
    }
}

#[tokio::test]
async fn a_broken_legacy_sse_stream_fails_the_pending_call() {
    let server = serve_legacy().await;
    let mut client = support::client_for(
        support::runtime(),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let error = call_tool(
        &mut client,
        &call("stream-fail"),
        None::<&RecordingHost>,
        RAISE,
    )
    .await
    .unwrap_err();
    match &error {
        SessionError::Stream(failure) => {
            assert_eq!(failure.kind(), StreamFailureKind::ValueError);
        }
        other => panic!("expected a stream failure, got {other}"),
    }
}
