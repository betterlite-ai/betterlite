#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, IntoFuture as _};
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap as HttpHeaders;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;
use betterlite_http::proxy::ProxiedClient;
use futures_util::StreamExt as _;
use rstest::rstest;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::sleep;

use super::convert::to_compat_stream_response;
use super::error::ClientError;
use super::records::SendMessageResponse;
use super::records::compat::SendMessageRequest;
use super::records::core::CoreStreamEvent;
use super::retry::{SendError, SendTarget, execute_send_with_retry, execute_stream_with_retry};
use super::send::send_message;
use super::{
    A2AClient, A2AClientSettings, DEFAULT_A2A_AGENT_TIMEOUT, agent_card_path, create_a2a_client,
};
use crate::agent_card::{AgentCard, WireValue};
use crate::card_resolver::card_url::get_agent_card_url;

const CARD_PATH: &str = "/.well-known/agent-card.json";
const TEAM_HEADER: &str = "x-team";

#[derive(Clone)]
enum Reply {
    Json(String),
    Sse(Vec<String>),
    Slow(Duration, String),
}

#[derive(Clone)]
struct Seen {
    path: String,
    team: Option<String>,
}

#[derive(Clone)]
struct AgentState {
    card: String,
    reply: Reply,
    seen: Arc<Mutex<Vec<Seen>>>,
}

fn router(state: AgentState) -> Router {
    Router::new()
        .route(CARD_PATH, get(card))
        .fallback(post(message))
        .with_state(state)
}

fn record(state: &AgentState, path: &str, headers: &HttpHeaders) {
    let team = headers
        .get(TEAM_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    state.seen.lock().unwrap().push(Seen {
        path: path.to_owned(),
        team,
    });
}

async fn card(State(state): State<AgentState>, headers: HttpHeaders) -> Response {
    record(&state, CARD_PATH, &headers);
    ([(CONTENT_TYPE, "application/json")], state.card).into_response()
}

async fn message(
    State(state): State<AgentState>,
    headers: HttpHeaders,
    _request: Bytes,
) -> Response {
    record(&state, "/", &headers);
    match state.reply {
        Reply::Json(reply) => ([(CONTENT_TYPE, "application/json")], reply).into_response(),
        Reply::Sse(frames) => {
            ([(CONTENT_TYPE, "text/event-stream")], frames.concat()).into_response()
        }
        Reply::Slow(delay, reply) => {
            sleep(delay).await;
            ([(CONTENT_TYPE, "application/json")], reply).into_response()
        }
    }
}

async fn start_agent() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (listener, base)
}

async fn run_agent<F>(
    listener: TcpListener,
    state: AgentState,
    scenario: F,
) -> (F::Output, Vec<Seen>)
where
    F: Future,
{
    let app = router(state.clone());
    let (stop, stopped) = oneshot::channel::<()>();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            stopped.await.unwrap_or_default();
        })
        .into_future();
    let run = async move {
        let outcome = scenario.await;
        stop.send(()).unwrap_or_default();
        outcome
    };
    let (outcome, drained) = tokio::join!(run, server);
    drained.unwrap();
    let seen = state.seen.lock().unwrap().clone();
    (outcome, seen)
}

fn unused_base() -> String {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

fn pooled_client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn card_json(endpoint: &str, streaming: bool) -> String {
    format!(
        r#"{{"name":"fake-agent","url":"{endpoint}","version":"1.0","capabilities":{{"streaming":{streaming}}},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[]}}"#
    )
}

fn settings(base: &str, streaming: bool, timeout: Duration) -> A2AClientSettings {
    A2AClientSettings {
        base_url: base.to_owned(),
        timeout,
        extra_headers: BTreeMap::from([(TEAM_HEADER.to_owned(), "blue".to_owned())]),
        streaming,
        relative_card_path: None,
    }
}

fn send_request(text: &str) -> SendMessageRequest {
    let body = format!(
        r#"{{"id":"request-1","params":{{"message":{{"kind":"message","messageId":"m-1","role":"user","parts":[{{"kind":"text","text":"{text}"}}]}}}}}}"#
    );
    serde_json::from_str(&body).unwrap()
}

fn buffered_message(text: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":"rpc-1","result":{{"message":{{"messageId":"m-1","role":"ROLE_AGENT","parts":[{{"text":"{text}"}}]}}}}}}"#
    )
}

fn buffered_task() -> String {
    String::from(
        r#"{"jsonrpc":"2.0","id":"rpc-1","result":{"task":{"id":"t-1","contextId":"c-1","status":{"state":"TASK_STATE_COMPLETED"}}}}"#,
    )
}

fn sse_frame(payload: &str) -> String {
    format!("data: {payload}\n\n")
}

fn message_frame(text: &str) -> String {
    sse_frame(&buffered_message(text))
}

fn status_frame() -> String {
    sse_frame(
        r#"{"jsonrpc":"2.0","id":"rpc-1","result":{"statusUpdate":{"taskId":"t-1","contextId":"c-1","status":{"state":"TASK_STATE_WORKING"}}}}"#,
    )
}

fn team_of(seen: &[Seen], path: &str) -> Option<String> {
    seen.iter()
        .find(|entry| entry.path == path)
        .and_then(|entry| entry.team.clone())
}

fn card_name(card: &AgentCard) -> Option<&str> {
    match card.name.as_ref()?.as_ref()? {
        WireValue::Typed(name) => Some(name.as_str()),
        WireValue::Passthrough(_) => None,
    }
}

fn target_for(client: &A2AClient, api_base: Option<String>) -> SendTarget {
    SendTarget {
        agent_card: Some(client.agent_card.clone()),
        card_url: get_agent_card_url(&client.agent_card).map(str::to_owned),
        api_base,
        agent_name: Some("fake-agent".to_owned()),
    }
}

#[rstest]
#[case::retains_handles_and_headers(true)]
#[case::without_extra_headers_has_no_context(false)]
#[tokio::test]
async fn a2a_client_creation(#[case] with_headers: bool) {
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Json(buffered_message("hi")),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let mut settings = settings(&base, true, Duration::from_secs(30));
    if !with_headers {
        settings.extra_headers = BTreeMap::new();
    }
    let (created, seen) = run_agent(listener, state, async move {
        create_a2a_client(&pool, settings).await
    })
    .await;
    let client = created.unwrap();
    assert_eq!(card_name(&client.agent_card), Some("fake-agent"));
    assert_eq!(
        client.http_client.limits.read_timeout,
        Duration::from_secs(30)
    );
    assert!(client.streaming);
    assert_eq!(client.endpoint.as_str(), format!("{base}/"));
    if with_headers {
        let context = client.call_context.unwrap();
        assert_eq!(
            context
                .service_parameters
                .unwrap()
                .get(TEAM_HEADER)
                .map(String::as_str),
            Some("blue")
        );
        assert_eq!(team_of(&seen, CARD_PATH).as_deref(), Some("blue"));
        let params = BTreeMap::from([(
            "agent_card_path".to_owned(),
            AgentJson::String("custom/card.json".to_owned()),
        )]);
        assert_eq!(
            agent_card_path(&params),
            Some("custom/card.json".to_owned())
        );
    } else {
        assert!(client.call_context.is_none());
        assert_eq!(team_of(&seen, CARD_PATH), None);
        assert_eq!(agent_card_path(&BTreeMap::new()), None);
    }
    let empty = BTreeMap::from([(
        "agent_card_path".to_owned(),
        AgentJson::String(String::new()),
    )]);
    assert_eq!(agent_card_path(&empty), None);
    let wrong = BTreeMap::from([("agent_card_path".to_owned(), AgentJson::Bool(true))]);
    assert_eq!(agent_card_path(&wrong), None);
}

#[rstest]
#[case::buffered_message(false, "message")]
#[case::buffered_task(false, "task")]
#[case::buffered_without_result(false, "empty")]
#[case::streaming_empty_events(true, "empty-stream")]
#[case::streaming_status_last(true, "status")]
#[tokio::test]
async fn native_send_behavior(#[case] streaming: bool, #[case] scenario: &str) {
    let (listener, base) = start_agent().await;
    let reply = match scenario {
        "task" => Reply::Json(buffered_task()),
        "empty" => Reply::Json(String::from(
            r#"{"jsonrpc":"2.0","id":"rpc-1","result":{}}"#,
        )),
        "empty-stream" => Reply::Sse(Vec::new()),
        "status" => Reply::Sse(vec![status_frame()]),
        _ => Reply::Json(buffered_message("hello")),
    };
    let state = AgentState {
        card: card_json(&base, true),
        reply,
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (outcome, seen) = run_agent(listener, state, async move {
        let client = create_a2a_client(&pool, settings(&base, streaming, Duration::from_secs(5)))
            .await
            .unwrap();
        send_message(&client, &request).await
    })
    .await;
    match scenario {
        "message" | "task" => assert_eq!(
            outcome
                .unwrap()
                .result
                .get("kind")
                .and_then(AgentJson::as_str),
            Some("message")
        ),
        "empty" | "empty-stream" => assert!(matches!(
            outcome,
            Err(ClientError::Final { message }) if message.contains("no response")
        )),
        "status" => assert!(matches!(
            outcome,
            Err(ClientError::Final { message }) if message.contains("final event")
        )),
        other => panic!("unknown scenario {other}"),
    }
    assert_eq!(team_of(&seen, "/").as_deref(), Some("blue"));

    let (empty_listener, empty_base) = start_agent().await;
    let empty_state = AgentState {
        card: card_json(&empty_base, true),
        reply: Reply::Sse(Vec::new()),
        seen: Arc::default(),
    };
    let empty_pool = pooled_client();
    let (accepted, _empty_seen) = run_agent(
        empty_listener,
        empty_state,
        Box::pin(async move {
            let client = create_a2a_client(
                &empty_pool,
                settings(&empty_base, true, Duration::from_secs(5)),
            )
            .await
            .unwrap();
            let target = target_for(&client, Some(empty_base));
            execute_stream_with_retry(client, send_request("hello"), target)
                .collect::<Vec<_>>()
                .await
        }),
    )
    .await;
    assert!(accepted.is_empty());
}

enum RetryOutcome {
    Send(Box<Result<SendMessageResponse, SendError>>),
    Stream(Vec<Result<SendMessageResponse, SendError>>),
}

#[rstest]
#[case::send_retries_localhost_url("send-retry")]
#[case::send_stops_after_two_attempts("send-give-up")]
#[case::stream_retries_before_first_chunk("stream-retry")]
#[case::stream_propagates_after_first_chunk("stream-propagate")]
#[tokio::test]
async fn native_send_retry(#[case] scenario: &str) {
    let (listener, base) = start_agent().await;
    let dead = unused_base();
    let other = unused_base();
    let streaming = scenario.starts_with("stream");
    let card_url = if scenario == "stream-propagate" {
        base.clone()
    } else {
        dead
    };
    let reply = if scenario == "stream-propagate" {
        Reply::Sse(vec![
            message_frame("first"),
            String::from("data: not-json\n\n"),
        ])
    } else if streaming {
        Reply::Sse(vec![message_frame("chunk")])
    } else {
        Reply::Json(buffered_message("retried"))
    };
    let state = AgentState {
        card: card_json(&card_url, true),
        reply,
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let api_base = if scenario == "send-give-up" {
        other
    } else {
        base.clone()
    };
    let request = send_request("hello");
    let (outcome, seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client =
                create_a2a_client(&pool, settings(&base, streaming, Duration::from_secs(5)))
                    .await
                    .unwrap();
            if streaming {
                let target = target_for(&client, Some(api_base));
                RetryOutcome::Stream(
                    execute_stream_with_retry(client, request, target)
                        .collect::<Vec<_>>()
                        .await,
                )
            } else {
                let mut target = target_for(&client, Some(api_base));
                RetryOutcome::Send(Box::new(
                    execute_send_with_retry(client, &request, &mut target).await,
                ))
            }
        }),
    )
    .await;
    match outcome {
        RetryOutcome::Send(result) => {
            if scenario == "send-give-up" {
                assert!(matches!(*result, Err(SendError::NoResponse)));
                assert_eq!(seen.len(), 1);
            } else {
                assert!(result.is_ok());
                assert_eq!(seen.len(), 2);
                assert_eq!(team_of(&seen, "/").as_deref(), Some("blue"));
            }
        }
        RetryOutcome::Stream(chunks) => {
            if scenario == "stream-propagate" {
                assert_eq!(chunks.len(), 2);
                assert!(chunks.first().unwrap().is_ok());
                assert!(matches!(&chunks[1], Err(SendError::Client(_))));
                assert_eq!(seen.len(), 2);
            } else {
                assert_eq!(chunks.len(), 1);
                assert!(chunks.first().unwrap().is_ok());
                assert_eq!(seen.len(), 2);
                assert_eq!(team_of(&seen, "/").as_deref(), Some("blue"));
            }
        }
    }
}

#[rstest]
#[case::default_timeout(0)]
#[case::configured_timeout(50)]
#[tokio::test]
async fn client_timeout(#[case] timeout_ms: u64) {
    assert_eq!(
        DEFAULT_A2A_AGENT_TIMEOUT.as_secs(),
        6000,
        "the A2A default timeout is 6000 seconds"
    );
    assert_eq!(DEFAULT_A2A_AGENT_TIMEOUT.as_secs(), 6000);
    if timeout_ms == 0 {
        return;
    }
    let (listener, base) = start_agent().await;
    let state = AgentState {
        card: card_json(&base, true),
        reply: Reply::Slow(Duration::from_millis(500), buffered_message("late")),
        seen: Arc::default(),
    };
    let pool = pooled_client();
    let request = send_request("hello");
    let (outcome, _seen) = run_agent(
        listener,
        state,
        Box::pin(async move {
            let client = create_a2a_client(
                &pool,
                settings(&base, false, Duration::from_millis(timeout_ms)),
            )
            .await
            .unwrap();
            send_message(&client, &request).await
        }),
    )
    .await;
    assert!(matches!(outcome, Err(ClientError::Timeout { .. })));
}

#[test]
fn compat_stream_response_round_trip() {
    let message = serde_json::from_str(
        r#"{"messageId":"m-1","role":"ROLE_AGENT","parts":[{"text":"hello"}]}"#,
    )
    .unwrap();
    let response = to_compat_stream_response(
        CoreStreamEvent::Message(message),
        serde_json::from_str::<JsonRpcId>(r#""request-1""#).unwrap(),
    )
    .unwrap();
    let value = &response.result;
    assert_eq!(response.jsonrpc, "2.0");
    assert_eq!(
        response.id,
        serde_json::from_str::<JsonRpcId>(r#""request-1""#).unwrap()
    );
    let json = serde_json::to_string(&value).unwrap();
    assert!(json.contains(r#""kind":"message""#));
    assert!(json.contains(r#""role":"agent""#));
}
