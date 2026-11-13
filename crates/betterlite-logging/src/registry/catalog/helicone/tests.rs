#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::adapter::HeliconeAdapter;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::helicone::initialization::{HeliconeLogger, Settings};
use crate::registry::catalog::{CatalogInputs, LoggerCatalog};

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
    "metadata": {"user_api_key_user_id": "user-1", "requester_metadata": {"helicone_property_team": "core"}},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {
        "id": "chatcmpl-1",
        "choices": [{"message": {"role": "assistant", "content": "Hi"}}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
    }
}"#;

fn inputs(environment: &[(&str, &str)]) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    CatalogInputs {
        config: Arc::new(
            ConfigLoader {
                lookup: BTreeMap::new(),
            }
            .load("")
            .expect("valid config"),
        ),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(
            environment
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<String, String>>(),
        ),
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

fn adapter(base: &str, key: Option<&str>) -> HeliconeAdapter {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    HeliconeAdapter {
        logger: Arc::new(HeliconeLogger {
            settings: Settings {
                api_key: key.map(SecretString::from),
                api_base: base.to_owned(),
            },
            client: ProxiedClient::outbound(&policy).expect("http client"),
            timeout: Duration::from_secs(5),
        }),
        tracker: TaskTracker::new(),
    }
}

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    match value {
        Json::Object(fields) => fields
            .get(name)
            .unwrap_or_else(|| panic!("the body carries {name}")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the value is an object")
        }
    }
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

struct Received {
    request_line: String,
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
    let request_line = lines.next().expect("a request line").to_owned();
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
        request_line,
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

#[tokio::test]
async fn resolves_by_name_to_a_helicone_logger() {
    let catalog = LoggerCatalog::new(inputs(&[("HELICONE_API_KEY", "helicone-key")]));
    let logger = catalog
        .resolve_logger("helicone")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "helicone");
}

#[tokio::test]
async fn a_sync_success_event_posts_the_mapped_envelope() {
    let (base, served) = serve_once().await;
    let adapter = adapter(&base, Some("helicone-key"));
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    adapter.tracker.close();
    adapter.tracker.wait().await;

    let received = timeout(Duration::from_secs(5), served)
        .await
        .expect("the envelope is posted");
    assert_eq!(received.request_line, "POST /oai/v1/log HTTP/1.1");
    assert_eq!(
        received.headers.get("authorization").map(String::as_str),
        Some("Bearer helicone-key")
    );
    let body: Json = serde_json::from_slice(&received.body).expect("the posted body is JSON");
    assert_mapped_envelope(&body);
}

fn assert_mapped_envelope(body: &Json) {
    let request = field(body, "providerRequest");
    assert_eq!(
        field(field(request, "json"), "model"),
        &Json::Text("gpt-4o".to_owned())
    );
    assert_eq!(
        field(field(request, "json"), "messages"),
        &Json::List(vec![Json::Object(IndexMap::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("Hello".to_owned())),
        ]))])
    );
    assert_eq!(
        field(field(request, "meta"), "Helicone-Auth"),
        &Json::Text("Bearer helicone-key".to_owned())
    );
    assert_eq!(
        field(field(request, "meta"), "helicone_property_team"),
        &Json::Text("core".to_owned())
    );
    let timing = field(body, "timing");
    assert_eq!(
        field(field(timing, "startTime"), "seconds"),
        &number(1_704_067_200)
    );
    assert_eq!(
        field(field(timing, "endTime"), "milliseconds"),
        &number(500)
    );
    assert_eq!(
        field(field(body, "providerResponse"), "status"),
        &number(200)
    );
}

#[tokio::test]
async fn a_vertex_ai_provider_routes_a_claude_model_to_the_custom_endpoint() {
    let (base, served) = serve_once().await;
    let adapter = adapter(&base, Some("helicone-key"));
    let fixture = PAYLOAD.replace(
        r#""model": "gpt-4o","#,
        r#""model": "claude-3-5-sonnet@20240620",
    "custom_llm_provider": "vertex_ai","#,
    );
    adapter
        .log_success_event(&event_from(&fixture))
        .expect("the event is accepted");
    adapter.tracker.close();
    adapter.tracker.wait().await;

    let received = timeout(Duration::from_secs(5), served)
        .await
        .expect("the envelope is posted");
    assert_eq!(received.request_line, "POST /custom/v1/log HTTP/1.1");
    let body: Json = serde_json::from_slice(&received.body).expect("the posted body is JSON");
    let request = field(&body, "providerRequest");
    assert_eq!(
        field(request, "url"),
        &Json::Text("https://aiplatform.googleapis.com/v1".to_owned())
    );
    // a vertex_ai model keeps its name and skips the Claude response mapping
    assert_eq!(
        field(field(request, "json"), "model"),
        &Json::Text("claude-3-5-sonnet@20240620".to_owned())
    );
}

#[tokio::test]
async fn an_async_success_event_posts_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    let adapter = adapter(&format!("http://{address}"), Some("helicone-key"));
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the async event is ignored");
    let accepted = timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(accepted.is_err(), "the async hook must not post");
}

#[tokio::test]
async fn a_missing_api_key_posts_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    let adapter = adapter(&format!("http://{address}"), None);
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    adapter.tracker.close();
    adapter.tracker.wait().await;
    let accepted = timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(accepted.is_err(), "a keyless logger must not post");
}

#[tokio::test]
async fn a_payload_without_usable_times_is_a_delivery_error() {
    let payload = PAYLOAD.replace(
        r#""endTime": 1704067201.5"#,
        r#""endTime": 999999999999999999"#,
    );
    let event = LoggerEvent {
        payload: Arc::new(serde_json::from_str(&payload).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    };
    let adapter = adapter("http://127.0.0.1:1", Some("helicone-key"));
    let error = adapter
        .log_success_event(&event)
        .expect_err("the timestamp is rejected");
    assert!(error.to_string().contains("endTime"));
}
