#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{build, build_callback};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::logging_pre_post::env::{DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv};
use crate::logging_pre_post::hooks::InputCallback;
use crate::logging_pre_post::pre_call::{PreCall, pre_call};
use crate::logging_privacy::request::Decision;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::updates::{CallUpdate, update_environment_variables};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

const SUCCESS_RESPONSE: &str = r#"{"id": "chatcmpl-1", "model": "gpt-4o-2024-08-06", "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}}]}"#;

const TEMPLATE: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "completion",
    "stream": false,
    "response_cost": 0.0000125,
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
    "metadata": {"user_api_key_user_id": "user-1"},
    "end_user": "proxy-user-1",
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {"user": "end-user-1"},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": RESPONSE,
    "error_str": ERROR_STR
}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture JSON decodes")
}

fn payload_text(response: &str, error: Option<&str>) -> String {
    let error = error.map_or_else(
        || "null".to_owned(),
        |message| serde_json::to_string(message).expect("the error encodes as a JSON string"),
    );
    TEMPLATE
        .replace("RESPONSE", response)
        .replace("ERROR_STR", &error)
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

fn success_event() -> LoggerEvent {
    event_from(&payload_text(SUCCESS_RESPONSE, None))
}

fn failure_event() -> LoggerEvent {
    event_from(&payload_text("null", Some("upstream returned 500")))
}

fn environment_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn inputs(environment: &[(&str, &str)], config: &str) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(config)
    .expect("the fixture config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(environment_map(environment)),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn configured_inputs(environment: &[(&str, &str)]) -> CatalogInputs {
    inputs(environment, "")
}

const CONFIGURED_TABLE: &str = "callback_settings:\n  supabase:\n    table_name: custom_logs\n";

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
    let response = "HTTP/1.1 201 Created\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    stream
        .write_all(response.as_bytes())
        .await
        .expect("the response write");
    stream.flush().await.expect("the response flush");
}

fn body_of(received: &Received) -> Json {
    serde_json::from_slice(&received.body).expect("the body is JSON")
}

fn header<'headers>(received: &'headers Received, name: &str) -> &'headers str {
    received
        .headers
        .get(name)
        .map(String::as_str)
        .unwrap_or_else(|| panic!("the request carries {name}"))
}

#[test]
fn resolves_by_name_to_a_supabase_logger() {
    let catalog = LoggerCatalog::new(configured_inputs(&[
        ("SUPABASE_URL", "https://project.supabase.test"),
        ("SUPABASE_KEY", "service-key"),
    ]));
    let logger = catalog
        .resolve_logger("supabase")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "supabase");
}

#[test]
fn missing_credentials_give_lite_llms_init_message() {
    let error = build(&configured_inputs(&[("SUPABASE_KEY", "service-key")]))
        .err()
        .expect("the url is required");
    assert!(matches!(
        error,
        CatalogError::Init { message }
            if message == "LiteLLM Error, trying to use Supabase but url or key not passed. Create a table and set `litellm.supabase_url=<your-url>` and `litellm.supabase_key=<your-key>`"
    ));
}

#[test]
fn an_empty_credential_gives_init() {
    assert!(matches!(
        build(&configured_inputs(&[
            ("SUPABASE_URL", "https://project.supabase.test"),
            ("SUPABASE_KEY", ""),
        ])),
        Err(CatalogError::Init { .. })
    ));
}

#[test]
fn a_url_that_is_not_http_gives_init() {
    assert!(matches!(
        build(&configured_inputs(&[
            ("SUPABASE_URL", "ftp://project.supabase.test"),
            ("SUPABASE_KEY", "service-key"),
        ])),
        Err(CatalogError::Init { message })
            if message == "SUPABASE_URL must use the http or https scheme"
    ));
}

#[test]
fn an_unconfigured_supabase_is_skipped() {
    let catalog = LoggerCatalog::new(configured_inputs(&[]));
    assert!(
        catalog
            .resolve_logger("supabase")
            .expect("the name is known")
            .is_none()
    );
}

fn success_body() -> Json {
    json(
        r#"{
            "response_time": 1.5,
            "model": "gpt-4o-2024-08-06",
            "total_cost": 0.0000125,
            "messages": [{"role": "user", "content": "Hello"}],
            "response": "hi",
            "end_user": "end-user-1",
            "litellm_call_id": "call-1",
            "status": "success"
        }"#,
    )
}

#[tokio::test]
async fn callback_logger_success_writes_to_the_configured_table() {
    let (base, served) = serve_once().await;
    let catalog = LoggerCatalog::new(configured_inputs(&[
        ("SUPABASE_URL", &base),
        ("SUPABASE_KEY", "service-key"),
    ]));
    let logger = catalog
        .resolve_logger("supabase")
        .expect("the name is available")
        .expect("the logger builds");
    logger
        .log_success_event(&success_event())
        .expect("the event is accepted");
    let received = served.await;
    assert_eq!(received.method, "POST");
    assert_eq!(
        received.path,
        "/rest/v1/request_logs?on_conflict=litellm_call_id"
    );
    assert_eq!(header(&received, "apikey"), "service-key");
    assert_eq!(header(&received, "authorization"), "Bearer service-key");
    assert_eq!(
        header(&received, "prefer"),
        "resolution=merge-duplicates,return=representation"
    );
    assert_eq!(body_of(&received), success_body());
}

#[tokio::test]
async fn callback_logger_failure_writes_the_failure_row() {
    let (base, served) = serve_once().await;
    let catalog = LoggerCatalog::new(configured_inputs(&[
        ("SUPABASE_URL", &base),
        ("SUPABASE_KEY", "service-key"),
    ]));
    let logger = catalog
        .resolve_logger("supabase")
        .expect("the name is available")
        .expect("the logger builds");
    logger
        .log_failure_event(&failure_event())
        .expect("the event is accepted");
    let received = served.await;
    assert_eq!(received.method, "POST");
    assert_eq!(
        received.path,
        "/rest/v1/request_logs?on_conflict=litellm_call_id"
    );
    assert_eq!(
        body_of(&received),
        json(
            r#"{
                "response_time": 1.5,
                "model": "gpt-4o",
                "total_cost": 0.0000125,
                "messages": [{"role": "user", "content": "Hello"}],
                "error": "upstream returned 500",
                "end_user": "end-user-1",
                "litellm_call_id": "call-1",
                "status": "failure"
            }"#
        )
    );
}

#[tokio::test]
async fn callback_logger_writes_the_overridden_table_name() {
    let (base, served) = serve_once().await;
    let catalog = LoggerCatalog::new(inputs(
        &[("SUPABASE_URL", &base), ("SUPABASE_KEY", "service-key")],
        CONFIGURED_TABLE,
    ));
    let logger = catalog
        .resolve_logger("supabase")
        .expect("the name is available")
        .expect("the logger builds");
    logger
        .log_success_event(&success_event())
        .expect("the event is accepted");
    let received = served.await;
    assert_eq!(
        received.path,
        "/rest/v1/custom_logs?on_conflict=litellm_call_id"
    );
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn pre_call_state(user: Option<&str>) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{
            "model": "gpt-4o",
            "call_type": "completion",
            "start_time": "2026-01-01T00:00:00+00:00",
            "litellm_call_id": "call-1",
            "function_id": "fn-1",
            "messages": [{"role": "user", "content": "Hello"}]
        }"#,
    )
    .expect("the fixture init decodes");
    let mut state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated".to_owned(),
        std::time::Duration::default,
    )
    .expect("the fixture state builds");
    update_environment_variables(
        &mut state,
        &IndexMap::new(),
        CallUpdate {
            model: None,
            user: user.map(str::to_owned),
            optional_params: IndexMap::new(),
            additional_params: IndexMap::new(),
        },
    )
    .expect("the fixture update succeeds");
    state
}

async fn initiated_body(user: Option<&str>) -> Json {
    let (base, served) = serve_once().await;
    let callback = build_callback(&configured_inputs(&[
        ("SUPABASE_URL", &base),
        ("SUPABASE_KEY", "service-key"),
    ]))
    .expect("the callback builds");
    let callbacks = [InputCallback::InputEvent(&*callback)];
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles");
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &Silent,
        input_callbacks: &callbacks,
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .expect("the fixture time parses"),
    };
    let call: PreCall =
        serde_json::from_str(r#"{"input": [{"role": "user", "content": "Hello"}]}"#)
            .expect("the fixture call decodes");
    pre_call(&mut pre_call_state(user), &call, &env).expect("the pre-call hook runs");
    let received = served.await;
    assert_eq!(received.method, "POST");
    assert_eq!(received.path, "/rest/v1/request_logs");
    body_of(&received)
}

#[tokio::test]
async fn callback_logger_pre_call_hook_writes_the_initiated_row() {
    assert_eq!(
        initiated_body(Some("alice")).await,
        json(
            r#"{
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "Hello"}],
                "end_user": "alice",
                "status": "initiated",
                "litellm_call_id": "call-1"
            }"#
        )
    );
}

#[tokio::test]
async fn callback_logger_pre_call_hook_writes_a_null_end_user_without_a_user() {
    let body = initiated_body(None).await;
    let Json::Object(fields) = body else {
        panic!("the initiated row is an object");
    };
    assert_eq!(fields.get("end_user"), Some(&Json::Null));
}
