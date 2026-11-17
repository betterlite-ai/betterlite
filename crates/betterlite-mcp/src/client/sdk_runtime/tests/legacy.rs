use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::stream::{self, StreamExt as _};
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{Sender, channel};
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::client::sdk_runtime::session::SdkSession;
use crate::client::sessions::error::SessionError;
use crate::client::sessions::run::run_with_session;
use crate::client::sessions::stream::StreamFailureKind;
use crate::client::sessions::transport::TransportType;
use crate::client::tools::calling::{CallToolOptions, call_tool};
use crate::client::tools::listing::list_tools;
use crate::client::tools::result::{ContentBlock, ToolCallResult};
use crate::tools::pagination::McpSession;
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
const ALLOW_INPUT: CallToolOptions = CallToolOptions {
    raise_on_error: true,
    allow_input_required: true,
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
struct LegacyServer {
    outbox: Mutex<Option<Sender<String>>>,
    initialized: AtomicBool,
    // set by a case that needs the handshake refused with a JSON-RPC error
    reject_initialize: AtomicBool,
    // the version the server answers the handshake with
    protocol_version: String,
}

impl LegacyServer {
    fn new(protocol_version: &str) -> Self {
        Self {
            outbox: Mutex::new(None),
            initialized: AtomicBool::new(false),
            reject_initialize: AtomicBool::new(false),
            protocol_version: protocol_version.to_owned(),
        }
    }

    fn stream(&self) -> Body {
        let (sender, receiver) = channel::<String>(64);
        let mut outbox = self.outbox.lock().unwrap_or_else(PoisonError::into_inner);
        let replaced = outbox.replace(sender);
        drop(replaced);
        drop(outbox);
        let opening = stream::once(async { Ok::<_, Infallible>(ENDPOINT_EVENT.to_owned()) });
        let rest = stream::unfold(receiver, |mut incoming| async move {
            let chunk = incoming.recv().await?;
            Some((Ok::<_, Infallible>(chunk), incoming))
        });
        Body::from_stream(opening.chain(rest))
    }

    fn push(&self, frame: String) {
        let outbox = self.outbox.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = outbox.as_ref() {
            drop(sender.try_send(frame));
        }
        drop(outbox);
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
    let Some(raw_id) = request.id.as_deref() else {
        if request.method == "notifications/initialized" {
            server.initialized.store(true, Ordering::SeqCst);
        }
        return StatusCode::ACCEPTED.into_response();
    };
    let id = raw_id.get().to_owned();
    let answer = match request.method.as_str() {
        "initialize" => Some(if server.reject_initialize.load(Ordering::SeqCst) {
            failure_reply(&id, "unsupported protocol")
        } else {
            initialize_reply(&server, &id)
        }),
        "tools/list" => Some(list_reply(&server, &id, request.params.as_deref())),
        "tools/call" => call_reply(&id, request.params.as_deref()).await,
        other => Some(failure_reply(&id, &format!("unknown method {other}"))),
    };
    let frame = answer.map_or_else(
        || BAD_EVENT.to_owned(),
        |reply| format!("data: {reply}\n\n"),
    );
    server.push(frame);
    StatusCode::ACCEPTED.into_response()
}

fn initialize_reply(server: &LegacyServer, id: &str) -> String {
    let version = &server.protocol_version;
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"{version}","capabilities":{{"tools":{{}},"prompts":{{}},"resources":{{}}}},"serverInfo":{{"name":"legacy-test","version":"1.0"}},"instructions":"{INSTRUCTIONS}"}}}}"#
    )
}

const FIRST_PAGE: &str = r#"{"tools":[{"name":"alpha","description":"the first tool","inputSchema":{"type":"object"}},{"name":"beta","description":"the second tool","inputSchema":{"type":"object"}}],"nextCursor":"page-2"}"#;
const SECOND_PAGE: &str = r#"{"tools":[{"name":"gamma","description":"the third tool","inputSchema":{"type":"object"}}]}"#;
const WRONG_RESULT: &str = r#"{"prompts":[]}"#;
// The tools page stays closed until the client sends notifications/initialized, as the SDK's own servers expect
fn list_reply(server: &LegacyServer, id: &str, params: Option<&RawValue>) -> String {
    if !server.initialized.load(Ordering::SeqCst) {
        return failure_reply(id, "notifications/initialized was not sent");
    }
    let page = match cursor(params).as_deref() {
        Some("page-2") => SECOND_PAGE,
        Some("wrong-result") => WRONG_RESULT,
        _ => FIRST_PAGE,
    };
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{page}}}"#)
}

fn cursor(params: Option<&RawValue>) -> Option<String> {
    let raw = params?;
    serde_json::from_str::<Cursored>(raw.get()).ok()?.cursor
}

async fn call_reply(id: &str, params: Option<&RawValue>) -> Option<String> {
    match tool_name(params).as_deref() {
        Some("echo") => Some(echo_reply(id, params)),
        Some("input-required") => Some(input_required_reply(id)),
        Some("stream-fail") => None,
        // the delay keeps this answer behind another session's stream failure on a shared runtime
        Some("rpc-error") => {
            sleep(Duration::from_millis(500)).await;
            Some(failure_reply(id, "bad arguments"))
        }
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

// The request line rides back in the text, so a case can read the params the client put on the wire
fn echo_reply(id: &str, params: Option<&RawValue>) -> String {
    let text = params.map_or_else(
        || "echoed".to_owned(),
        |raw| format!("echoed {}", raw.get()),
    );
    let Ok(encoded) = serde_json::to_string(&text) else {
        return failure_reply(id, "the echo text is not encodable");
    };
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"type":"text","text":{encoded}}}]}}}}"#
    )
}

fn failure_reply(id: &str, message: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32602,"message":"{message}"}}}}"#)
}

// A 2026-07-28 result, which rmcp's own server refuses to send to the older version this client negotiates
fn input_required_reply(id: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"input_required","requestState":"opaque"}}}}"#
    )
}

struct LegacySseServer {
    url: String,
    state: Arc<LegacyServer>,
    _tasks: JoinSet<Result<(), io::Error>>,
}

async fn serve_legacy() -> LegacySseServer {
    serve_legacy_with_version("2025-06-18").await
}

async fn serve_legacy_with_version(protocol_version: &str) -> LegacySseServer {
    let state = Arc::new(LegacyServer::new(protocol_version));
    let router = Router::new()
        .route("/sse", get(open_events))
        .route("/messages", post(receive_message))
        .with_state(Arc::clone(&state));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { axum::serve(listener, router).await });
    LegacySseServer {
        url: format!("http://{address}/sse"),
        state,
        _tasks: tasks,
    }
}

fn call(name: &str) -> CallRequest {
    let owned = name.to_owned();
    let arguments = None;
    CallRequest {
        name: owned,
        arguments,
    }
}

fn tool_names(tools: Vec<McpTool>) -> Vec<String> {
    tools.into_iter().map(|tool| tool.name).collect()
}

// The legacy echo tool reports the request line back in one text block
fn echo_text(result: &ToolCallResult) -> &str {
    let ToolCallResult::Complete(complete) = result else {
        panic!("expected a complete tool result");
    };
    let [ContentBlock::Text(text)] = complete.content.as_slice() else {
        panic!("unexpected content blocks");
    };
    &text.text
}

#[tokio::test]
async fn a_legacy_sse_session_lists_every_tool_page_and_calls_a_tool() {
    let server = serve_legacy().await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
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
    let arguments = Object::from([("city".to_owned(), Json::Text("Paris".to_owned()))]);
    let request = CallRequest {
        name: "echo".to_owned(),
        arguments: Some(arguments),
    };
    let result = Box::pin(call_tool(
        &mut client,
        &request,
        None::<&RecordingHost>,
        CallToolOptions::default(),
    ))
    .await
    .unwrap();
    let text = echo_text(&result);
    assert!(text.starts_with("echoed"), "{text}");
    assert!(text.contains(r#""city":"Paris""#), "{text}");
}

#[tokio::test]
async fn a_stream_failure_in_one_session_leaves_another_sessions_error_alone() {
    let broken = serve_legacy().await;
    let plain = serve_legacy().await;
    let runtime = Arc::new(support::runtime());
    let mut broken_client = support::client_on(
        Arc::clone(&runtime),
        support::settings(TransportType::Sse),
        &broken.url,
        support::CALL_TIMEOUT,
    );
    let mut plain_client = support::client_on(
        Arc::clone(&runtime),
        support::settings(TransportType::Sse),
        &plain.url,
        support::CALL_TIMEOUT,
    );
    let stream_fail = call("stream-fail");
    let rpc_error = call("rpc-error");
    let stream_call = call_tool(
        &mut broken_client,
        &stream_fail,
        None::<&RecordingHost>,
        RAISE,
    );
    let error_call = call_tool(&mut plain_client, &rpc_error, None::<&RecordingHost>, RAISE);
    let (streamed, answered) = tokio::join!(stream_call, error_call);
    assert!(matches!(streamed.unwrap_err(), SessionError::Stream(_)));
    let SessionError::Protocol(failure) = answered.unwrap_err() else {
        panic!("expected a protocol failure");
    };
    assert_eq!(failure.code, -32_602_i32);
    assert_eq!(failure.message, "bad arguments");
}

#[tokio::test]
async fn a_broken_legacy_sse_stream_fails_the_pending_call() {
    let server = serve_legacy().await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let error = Box::pin(call_tool(
        &mut client,
        &call("stream-fail"),
        None::<&RecordingHost>,
        RAISE,
    ))
    .await
    .unwrap_err();
    let SessionError::Stream(failure) = &error else {
        panic!("expected a stream failure, got {error}");
    };
    assert_eq!(failure.kind(), StreamFailureKind::ValueError);
}

#[tokio::test]
async fn an_initialize_error_keeps_its_json_rpc_code() {
    let server = serve_legacy().await;
    server.state.reject_initialize.store(true, Ordering::SeqCst);
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let error = list_tools(&mut client, support::CALL_TIMEOUT, true)
        .await
        .expect_err("the server refused the handshake");
    let SessionError::Protocol(failure) = error else {
        panic!("expected a protocol failure, got {error}");
    };
    assert_eq!(failure.code, -32_602_i32);
    assert_eq!(failure.message, "unsupported protocol");
}

// The SDK's handshake accepts only the versions it knows, so a server on a later one is a runtime failure
#[tokio::test]
async fn a_protocol_version_outside_the_handshake_fails_before_initialized() {
    let server = serve_legacy_with_version("2026-07-28").await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let error = list_tools(&mut client, support::CALL_TIMEOUT, true)
        .await
        .expect_err("the client refuses the version");
    assert!(matches!(error, SessionError::Runtime(_)), "{error}");
    assert!(!server.state.initialized.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_session_sends_initialized_before_it_lists_tools() {
    let server = serve_legacy().await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let tools = list_tools(&mut client, support::CALL_TIMEOUT, true)
        .await
        .unwrap();
    assert_eq!(tool_names(tools), ["alpha", "beta", "gamma"]);
}

#[tokio::test]
async fn a_result_of_the_wrong_kind_fails_the_listing() {
    let server = serve_legacy().await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let error = run_with_session(
        &mut client,
        async |session| <SdkSession as McpSession>::list_tools(session, Some("wrong-result")).await,
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, SessionError::Runtime(_)), "{error}");
}

#[tokio::test]
async fn an_input_required_result_is_an_error_unless_the_caller_allows_it() {
    let server = serve_legacy().await;
    let mut refused = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let error = Box::pin(call_tool(
        &mut refused,
        &call("input-required"),
        None::<&RecordingHost>,
        RAISE,
    ))
    .await
    .unwrap_err();
    assert!(matches!(error, SessionError::Runtime(_)), "{error}");

    let mut allowed = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Sse),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let result = Box::pin(call_tool(
        &mut allowed,
        &call("input-required"),
        None::<&RecordingHost>,
        ALLOW_INPUT,
    ))
    .await
    .unwrap();
    let ToolCallResult::InputRequired(required) = result else {
        panic!("expected an input-required result");
    };
    assert_eq!(required.request_state.as_deref(), Some("opaque"));
}
