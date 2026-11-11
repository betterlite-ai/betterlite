#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::adapter::LagoAdapter;
use super::build;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::finops::metering::lago::LagoLogger;
use crate::finops::metering::lago::settings::LagoSettings;
use crate::finops::metering::transport::{MeteringPost, MeteringResponse, MeteringSender};
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

const BASE: &str = "https://lago.test";

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {"user_api_key_user_id": "user-1", "user_api_key_team_id": "team-1"},
    "end_user": "end-user-1",
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {"id": "chatcmpl-1", "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}
}"#;

#[derive(Clone, Debug)]
struct Posted {
    url: String,
    api_key: String,
    content_type: String,
    body: Json,
}

#[derive(Clone, Default)]
struct FakeSender {
    posted: Arc<Mutex<Vec<Posted>>>,
}

impl FakeSender {
    fn posts(&self) -> Vec<Posted> {
        self.posted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake metering sender failed")]
struct Unreachable;

impl MeteringSender for FakeSender {
    type Error = Unreachable;

    fn post(
        &self,
        post: MeteringPost<'_>,
    ) -> impl Future<Output = Result<MeteringResponse, Unreachable>> + Send {
        let Ok(body) = serde_json::from_slice::<Json>(&post.body) else {
            return ready(Err(Unreachable));
        };
        self.posted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Posted {
                url: post.url.to_string(),
                api_key: post.bearer.expose_secret().to_owned(),
                content_type: post.content_type.to_owned(),
                body,
            });
        ready(Ok(MeteringResponse {
            status: 200,
            body: String::new(),
        }))
    }
}

fn environment_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn full_environment() -> BTreeMap<String, String> {
    environment_map(&[
        ("LAGO_API_KEY", "lago-key"),
        ("LAGO_API_BASE", BASE),
        ("LAGO_API_EVENT_CODE", "lago-code"),
    ])
}

fn settings(overrides: &[(&str, &str)]) -> LagoSettings {
    let mut values = full_environment();
    values.extend(environment_map(overrides));
    LagoSettings::from_environment(&values).expect("the fixture environment is complete")
}

fn inputs(environment: &[(&str, &str)]) -> CatalogInputs {
    inputs_with_config(environment, "")
}

fn inputs_with_config(environment: &[(&str, &str)], config: &str) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    CatalogInputs {
        config: Arc::new(
            ConfigLoader {
                lookup: BTreeMap::new(),
            }
            .load(config)
            .expect("valid config"),
        ),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(environment_map(environment)),
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn event_from(payload: &str) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(serde_json::from_str(payload).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

fn event() -> LoggerEvent {
    event_from(PAYLOAD)
}

fn adapter(settings: LagoSettings, sender: FakeSender) -> (LagoAdapter<FakeSender>, TaskTracker) {
    let tracker = TaskTracker::new();
    (
        LagoAdapter {
            logger: Arc::new(LagoLogger { settings, sender }),
            tracker: tracker.clone(),
        },
        tracker,
    )
}

fn field<'body>(body: &'body Json, name: &str) -> &'body Json {
    let Json::Object(fields) = body else {
        panic!("the posted body is a JSON object");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the posted body carries {name}"))
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

#[test]
fn resolves_by_name_to_a_lago_logger() {
    let catalog = LoggerCatalog::new(inputs(&[
        ("LAGO_API_KEY", "lago-key"),
        ("LAGO_API_BASE", BASE),
        ("LAGO_API_EVENT_CODE", "lago-code"),
    ]));
    let logger = catalog
        .resolve_logger("lago")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "lago");
}

#[test]
fn a_missing_key_gives_init() {
    assert!(matches!(
        build(&inputs(&[])),
        Err(CatalogError::Init { message })
            if message == "Missing keys=['LAGO_API_KEY', 'LAGO_API_BASE', 'LAGO_API_EVENT_CODE'] in environment."
    ));
}

#[test]
fn an_invalid_charge_by_gives_init() {
    assert!(matches!(
        build(&inputs(&[
            ("LAGO_API_KEY", "lago-key"),
            ("LAGO_API_BASE", BASE),
            ("LAGO_API_EVENT_CODE", "lago-code"),
            ("LAGO_API_CHARGE_BY", "organisation"),
        ])),
        Err(CatalogError::Init { message }) if message == "invalid LAGO_API_CHARGE_BY set"
    ));
}

#[test]
fn an_invalid_request_timeout_gives_init() {
    let inputs = inputs_with_config(
        &[
            ("LAGO_API_KEY", "lago-key"),
            ("LAGO_API_BASE", BASE),
            ("LAGO_API_EVENT_CODE", "lago-code"),
        ],
        "litellm_settings:\n  request_timeout: -1.0\n",
    );
    assert!(matches!(build(&inputs), Err(CatalogError::Init { .. })));
}

#[tokio::test]
async fn a_sync_success_event_posts_the_mapped_event() {
    let sender = FakeSender::default();
    let (adapter, tracker) = adapter(settings(&[]), sender.clone());
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let posts = sender.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, "https://lago.test/api/v1/events");
    assert_eq!(posts[0].api_key, "lago-key");
    assert_eq!(posts[0].content_type, "application/json");
    let body = &posts[0].body;
    let event_body = field(body, "event");
    assert_eq!(
        field(event_body, "external_subscription_id"),
        &Json::Text("end-user-1".to_owned())
    );
    assert_eq!(
        field(event_body, "code"),
        &Json::Text("lago-code".to_owned())
    );
    let properties = field(event_body, "properties");
    assert_eq!(field(properties, "model"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(
        serde_json::to_string(field(properties, "response_cost")).expect("a printable cost"),
        "0.5"
    );
    assert_eq!(field(properties, "prompt_tokens"), &number(2));
    assert_eq!(field(properties, "completion_tokens"), &number(3));
    assert_eq!(field(properties, "total_tokens"), &number(5));
}

#[tokio::test]
async fn an_async_success_event_posts_the_mapped_event() {
    let sender = FakeSender::default();
    let (adapter, _tracker) = adapter(settings(&[]), sender.clone());
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is logged");
    let posts = sender.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].api_key, "lago-key");
}

#[tokio::test]
async fn charge_by_team_id_selects_the_team() {
    let sender = FakeSender::default();
    let (adapter, tracker) = adapter(
        settings(&[("LAGO_API_CHARGE_BY", "team_id")]),
        sender.clone(),
    );
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let posts = sender.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        field(field(&posts[0].body, "event"), "external_subscription_id"),
        &Json::Text("team-1".to_owned())
    );
}

#[tokio::test]
async fn an_event_without_a_subscription_sends_nothing() {
    let sender = FakeSender::default();
    let (adapter, tracker) = adapter(settings(&[]), sender.clone());
    let payload = PAYLOAD.replace(r#""end_user": "end-user-1","#, r#""end_user": null,"#);
    adapter
        .log_success_event(&event_from(&payload))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(sender.posts().is_empty());
}

async fn properties_posted_for(payload: &str) -> Json {
    let sender = FakeSender::default();
    let (adapter, tracker) = adapter(settings(&[]), sender.clone());
    adapter
        .log_success_event(&event_from(payload))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let posts = sender.posts();
    assert_eq!(posts.len(), 1);
    field(field(&posts[0].body, "event"), "properties").clone()
}

fn has_field(body: &Json, name: &str) -> bool {
    let Json::Object(fields) = body else {
        panic!("the posted properties are a JSON object");
    };
    fields.contains_key(name)
}

#[tokio::test]
async fn a_call_that_is_not_a_completion_or_embedding_sends_no_usage() {
    let payload = PAYLOAD.replace(
        r#""call_type": "completion""#,
        r#""call_type": "image_generation""#,
    );
    let properties = properties_posted_for(&payload).await;
    assert!(!has_field(&properties, "prompt_tokens"));
    assert!(!has_field(&properties, "completion_tokens"));
    assert!(!has_field(&properties, "total_tokens"));
    assert_eq!(
        field(&properties, "model"),
        &Json::Text("gpt-4o".to_owned())
    );
}

#[rstest]
#[case::async_completion("acompletion")]
#[case::embedding("embedding")]
#[case::async_embedding("aembedding")]
#[tokio::test]
async fn an_async_completion_and_an_embedding_keep_their_usage(#[case] call_type: &str) {
    let payload = PAYLOAD.replace(
        r#""call_type": "completion""#,
        &format!(r#""call_type": "{call_type}""#),
    );
    let properties = properties_posted_for(&payload).await;
    assert_eq!(field(&properties, "total_tokens"), &number(5));
}

#[tokio::test]
async fn a_key_end_user_id_hides_the_request_user() {
    let sender = FakeSender::default();
    let (adapter, tracker) = adapter(settings(&[]), sender.clone());
    let payload = PAYLOAD.replace(
        r#""user_api_key_team_id": "team-1""#,
        r#""user_api_key_team_id": "team-1", "user_api_key_end_user_id": "key-end-user""#,
    );
    adapter
        .log_success_event(&event_from(&payload))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(sender.posts().is_empty());
}

#[tokio::test]
async fn an_empty_key_end_user_id_keeps_the_request_user() {
    let sender = FakeSender::default();
    let (adapter, tracker) = adapter(settings(&[]), sender.clone());
    let payload = PAYLOAD.replace(
        r#""user_api_key_team_id": "team-1""#,
        r#""user_api_key_team_id": "team-1", "user_api_key_end_user_id": """#,
    );
    adapter
        .log_success_event(&event_from(&payload))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let posts = sender.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        field(field(&posts[0].body, "event"), "external_subscription_id"),
        &Json::Text("end-user-1".to_owned())
    );
}

#[tokio::test]
async fn a_resolved_logger_posts_to_the_configured_base() {
    let (base, served) = serve_once().await;
    let catalog = LoggerCatalog::new(inputs(&[
        ("LAGO_API_KEY", "lago-key"),
        ("LAGO_API_BASE", &base),
        ("LAGO_API_EVENT_CODE", "lago-code"),
    ]));
    let logger = catalog
        .resolve_logger("lago")
        .expect("the name is available")
        .expect("the logger builds");
    logger
        .log_success_event(&event())
        .expect("the event is accepted");
    let received = served.await;
    assert_eq!(received.method, "POST");
    assert_eq!(received.path, "/api/v1/events");
    assert_eq!(
        received.headers.get("authorization").map(String::as_str),
        Some("Bearer lago-key")
    );
    let body: Json = serde_json::from_slice(&received.body).expect("the body is JSON");
    assert_eq!(
        field(field(&body, "event"), "code"),
        &Json::Text("lago-code".to_owned())
    );
}

struct Received {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

// The returned future accepts one request and answers it; the test joins it with the call under test.
async fn serve_once() -> (String, impl Future<Output = Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    let served = async move {
        let (mut stream, _) = listener.accept().await.expect("one request");
        let received = receive(&mut stream).await;
        answer(&mut stream).await;
        received
    };
    (format!("http://{address}"), served)
}

async fn receive(stream: &mut TcpStream) -> Received {
    let mut buffer = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let read = stream.read(&mut chunk).await.expect("a request read");
        assert!(read > 0, "the client closed before sending a request");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position.saturating_add(4);
        }
    };
    let head = String::from_utf8(buffer[..header_end].to_vec()).expect("a UTF-8 request head");
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().expect("a request line").split(' ');
    let method = request_line.next().expect("a method").to_owned();
    let path = request_line.next().expect("a path").to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().expect("a length"));
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0_u8; length.saturating_sub(body.len())];
        let read = stream.read(&mut chunk).await.expect("a body read");
        assert!(read > 0, "the client closed before sending the body");
        body.extend_from_slice(&chunk[..read]);
    }
    Received {
        method,
        path,
        headers,
        body,
    }
}

async fn answer(stream: &mut TcpStream) {
    let response = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    stream
        .write_all(response.as_bytes())
        .await
        .expect("the response write");
    stream.flush().await.expect("the response flush");
}
