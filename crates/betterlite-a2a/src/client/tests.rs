#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, IntoFuture as _};
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicU32, Ordering};
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
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::sleep;

use super::convert::{to_compat_stream_response, to_core_send_message_request};
use super::error::ClientError;
use super::records::compat::SendMessageRequest;
use super::records::core::{
    CoreArtifact, CoreArtifactUpdate, CorePart, CoreStatusUpdate, CoreStreamEvent, CoreTask,
    CoreTaskState, CoreTaskStatus, CoreTextPart,
};
use super::records::response::{SendMessageResponse, StreamResult};
use super::retry::{SendError, SendTarget, execute_send_with_retry, execute_stream_with_retry};
use super::send::send_message;
use super::{
    A2AClient, A2AClientSettings, DEFAULT_A2A_AGENT_TIMEOUT, RequestIds, agent_card_path,
    create_a2a_client,
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
    body: String,
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

fn record(state: &AgentState, path: &str, headers: &HttpHeaders, body: String) {
    let team = headers
        .get(TEAM_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    state.seen.lock().unwrap().push(Seen {
        path: path.to_owned(),
        team,
        body,
    });
}

async fn card(State(state): State<AgentState>, headers: HttpHeaders) -> Response {
    record(&state, CARD_PATH, &headers, String::new());
    ([(CONTENT_TYPE, "application/json")], state.card).into_response()
}

async fn message(
    State(state): State<AgentState>,
    headers: HttpHeaders,
    request: Bytes,
) -> Response {
    record(
        &state,
        "/",
        &headers,
        String::from_utf8(request.to_vec()).unwrap(),
    );
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
        extra_headers: BTreeMap::from([(TEAM_HEADER.to_owned(), SecretString::from("blue"))]),
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

fn configured_request(text: &str) -> SendMessageRequest {
    let body = format!(
        r#"{{"id":"request-1","params":{{"message":{{"kind":"message","messageId":"m-1","role":"user","parts":[{{"kind":"text","text":"{text}"}}]}},"configuration":{{"acceptedOutputModes":["text/plain"],"blocking":false,"historyLength":3,"pushNotificationConfig":{{"url":"https://hooks.example/"}}}}}}}}"#
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

#[derive(Default)]
struct CountingIds(AtomicU32);

impl RequestIds for CountingIds {
    fn next_request_id(&mut self) -> String {
        let next = self.0.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        format!("test-{next}")
    }
}

fn kind_of(response: &SendMessageResponse) -> Option<&'static str> {
    match &response.result {
        StreamResult::Message(_) => Some("message"),
        StreamResult::Task(_) => Some("task"),
        StreamResult::StatusUpdate(_) | StreamResult::ArtifactUpdate(_) => None,
    }
}

fn body_of(seen: &[Seen]) -> String {
    seen.iter()
        .find(|entry| entry.path == "/")
        .map(|entry| entry.body.clone())
        .unwrap_or_default()
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
                .map(ExposeSecret::expose_secret),
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
#[case::buffered_configuration(false, "configured")]
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
    let request = if scenario == "configured" {
        configured_request("hello")
    } else {
        send_request("hello")
    };
    let (outcome, seen) = run_agent(listener, state, async move {
        let mut client =
            create_a2a_client(&pool, settings(&base, streaming, Duration::from_secs(5)))
                .await
                .unwrap();
        client.request_ids = Arc::new(Mutex::new(Box::new(CountingIds::default())));
        send_message(&client, &request).await
    })
    .await;
    match scenario {
        "task" => assert_eq!(kind_of(&outcome.unwrap()), Some("task")),
        "message" | "configured" => assert_eq!(kind_of(&outcome.unwrap()), Some("message")),
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
    let body = body_of(&seen);
    let method = if streaming {
        "SendStreamingMessage"
    } else {
        "SendMessage"
    };
    assert!(body.contains(&format!("\"method\":\"{method}\"")));
    assert!(body.contains("\"id\":\"test-1\""));
    if matches!(scenario, "message" | "configured") {
        assert!(body.contains("\"messageId\":\"m-1\""));
        assert!(body.contains("\"role\":\"ROLE_USER\""));
        assert!(body.contains("\"text\":\"hello\""));
    }
    if scenario == "configured" {
        assert!(body.contains("\"acceptedOutputModes\":[\"text/plain\"]"));
        assert!(body.contains("\"historyLength\":3"));
        assert!(body.contains("\"returnImmediately\":true"));
        assert!(
            body.contains("\"taskPushNotificationConfig\":{\"url\":\"https://hooks.example/\"")
        );
    }

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
    );
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

fn multi_part_request() -> SendMessageRequest {
    serde_json::from_str(
        r#"{"id":"request-1","params":{"message":{"kind":"message","messageId":"m-1","role":"user","contextId":"c-1","parts":[{"kind":"text","text":"hi"},{"kind":"data","data":{"answer":42}},{"kind":"data","data":{"value":[1,2]},"metadata":{"data_part_compat":true}},{"kind":"file","file":{"bytes":"aGk=","mimeType":"text/plain","name":"a.txt"}},{"kind":"file","file":{"uri":"https://example/a.txt","mimeType":"text/plain"}}]}}}"#,
    )
    .unwrap()
}

fn request_id() -> JsonRpcId {
    let raw = r#""request-1""#;
    serde_json::from_str(raw).unwrap()
}

#[test]
fn compat_part_conversion_to_core() {
    let core = to_core_send_message_request(multi_part_request()).unwrap();
    assert!(matches!(&core.message.parts[0], CorePart::Text(_)));
    let CorePart::Data(data_part) = &core.message.parts[1] else {
        panic!("expected a data part");
    };
    assert_eq!(
        data_part.data,
        serde_json::from_str::<AgentJson>(r#"{"answer":42}"#).unwrap()
    );
    assert!(data_part.metadata.is_none());

    let CorePart::Data(unwrapped) = &core.message.parts[2] else {
        panic!("expected an unwrapped data part");
    };
    assert_eq!(
        unwrapped.data,
        serde_json::from_str::<AgentJson>("[1,2]").unwrap()
    );
    assert!(unwrapped.metadata.is_none());

    let CorePart::Raw(raw_part) = &core.message.parts[3] else {
        panic!("expected a raw file part");
    };
    assert_eq!(raw_part.raw, "aGk=");
    assert_eq!(raw_part.media_type.as_deref(), Some("text/plain"));
    assert_eq!(raw_part.filename.as_deref(), Some("a.txt"));

    let CorePart::Url(url_part) = &core.message.parts[4] else {
        panic!("expected a url file part");
    };
    assert_eq!(url_part.url, "https://example/a.txt");
}

#[test]
fn compat_part_conversion_to_compat() {
    let core = to_core_send_message_request(multi_part_request()).unwrap();
    let response = to_compat_stream_response(CoreStreamEvent::Message(core.message), request_id());
    let json = serde_json::to_string(&response.result).unwrap();
    assert!(json.contains(r#""kind":"message""#));
    assert!(json.contains(r#"{"kind":"text","text":"hi"}"#));
    assert!(json.contains(r#"{"kind":"data","data":{"answer":42}}"#));
    assert!(json.contains(
        r#"{"kind":"data","data":{"value":[1,2]},"metadata":{"data_part_compat":true}}"#
    ));
    assert!(json.contains(r#""bytes":"aGk=""#));
    assert!(json.contains(r#""uri":"https://example/a.txt""#));
}

#[test]
fn compat_part_conversion_errors() {
    let bad_bytes: SendMessageRequest = serde_json::from_str(
        r#"{"id":"request-1","params":{"message":{"messageId":"m-1","role":"user","parts":[{"kind":"file","file":{"bytes":"not base64!"}}]}}}"#,
    )
    .unwrap();
    assert!(matches!(
        to_core_send_message_request(bad_bytes),
        Err(super::convert::ConvertError::FileBytes(_))
    ));

    let missing_value: SendMessageRequest = serde_json::from_str(
        r#"{"id":"request-1","params":{"message":{"messageId":"m-1","role":"user","parts":[{"kind":"data","data":{"other":1},"metadata":{"data_part_compat":true}}]}}}"#,
    )
    .unwrap();
    assert!(matches!(
        to_core_send_message_request(missing_value),
        Err(super::convert::ConvertError::MissingDataValue)
    ));
}

#[test]
fn compat_task_conversion() {
    let task: CoreTask = serde_json::from_str(
        r#"{"id":"t-1","contextId":"c-1","status":{"state":"TASK_STATE_COMPLETED","message":{"messageId":"m-2","role":"ROLE_AGENT","parts":[{"text":"done"}]},"timestamp":"2026-01-01T00:00:00Z"},"history":[{"messageId":"m-1","role":"ROLE_USER","parts":[{"text":"hi"}]}],"artifacts":[{"artifactId":"a-1","name":"out","parts":[{"text":"x"}]}]}"#,
    )
    .unwrap();
    let response = to_compat_stream_response(CoreStreamEvent::Task(task), request_id());
    let json = serde_json::to_string(&response.result).unwrap();
    assert!(json.contains(r#""kind":"task""#));
    assert!(json.contains(r#""status":{"state":"completed","message":{"kind":"message""#));
    assert!(json.contains(r#""timestamp":"2026-01-01T00:00:00Z""#));
    assert!(json.contains(r#""history":[{"kind":"message""#));
    assert!(json.contains(
        r#""artifacts":[{"artifactId":"a-1","name":"out","parts":[{"kind":"text","text":"x"}]}]"#
    ));
}

#[test]
fn compat_status_update_conversion() {
    let status = CoreStatusUpdate {
        task_id: "t-1".to_owned(),
        context_id: Some("c-1".to_owned()),
        status: Some(CoreTaskStatus {
            state: CoreTaskState::Completed,
            message: None,
            timestamp: None,
        }),
        metadata: None,
    };
    let response = to_compat_stream_response(CoreStreamEvent::StatusUpdate(status), request_id());
    let json = serde_json::to_string(&response.result).unwrap();
    assert!(json.contains(r#""kind":"status-update""#));
    assert!(json.contains(r#""final":true"#));
    assert!(json.contains(r#""state":"completed""#));
    assert!(json.contains(r#""taskId":"t-1""#));
}

#[test]
fn compat_artifact_update_conversion() {
    let artifact = CoreArtifactUpdate {
        task_id: "t-1".to_owned(),
        context_id: Some("c-1".to_owned()),
        artifact: CoreArtifact {
            artifact_id: "a-1".to_owned(),
            name: Some("out".to_owned()),
            description: None,
            parts: vec![CorePart::Text(CoreTextPart {
                text: "x".to_owned(),
                metadata: None,
            })],
            metadata: None,
            extensions: Vec::new(),
        },
        append: Some(true),
        last_chunk: Some(true),
        metadata: None,
    };
    let response =
        to_compat_stream_response(CoreStreamEvent::ArtifactUpdate(artifact), request_id());
    let json = serde_json::to_string(&response.result).unwrap();
    assert!(json.contains(r#""kind":"artifact-update""#));
    assert!(json.contains(r#""append":true"#));
    assert!(json.contains(r#""lastChunk":true"#));
    assert!(json.contains(r#""artifact":{"artifactId":"a-1""#));
}
