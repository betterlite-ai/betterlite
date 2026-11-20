use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener as StdListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::realtime_contracts::config::{
    BaseRealtimeHttpConfig, Environment, RealtimeUrl,
};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, LogContext};
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};
use tokio::time::sleep;

use super::TransportDeps;
use super::calls::{CallsCall, session_data};
use super::client::{RealtimeClientSource, SuppliedClient};
use super::decode::decode_realtime_sdp;
use super::error::{RealtimeHttpError, StatusFailure};
use super::session::{SessionCall, SessionEndpoint, session_post};

mod wire;

const INIT: &str = r#"{"model":"realtime","call_type":"realtime","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
// sha256 over the two multipart parts in order: name, bytes, content type
const BOUNDARY: &str = "2f5cc9be1c163676e4ae2f347a1309d402542a6a431366f1f96b68a587ba2451";
// Three bytes of every cap: the SDP text is longer than the JSON body cap the decoder enforces
const TINY: ApiLimits = ApiLimits {
    max_json_bytes: 3,
    max_event_bytes: 3,
    max_items: 3,
};

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

static SINK: Sink = Sink;

struct Source {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
    created: AtomicUsize,
}

impl RealtimeClientSource for Source {
    fn client(&self) -> Result<ProxiedClient, ClientError> {
        self.created.fetch_add(1, Ordering::SeqCst);
        self.pool
            .client(&self.tls, &self.routing)
            .map_err(ClientError::Pool)
    }
}

fn test_source() -> Source {
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("model_list: []")
    .unwrap();
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    let tls = TlsPolicy::resolve(&config, &environment, None).unwrap();
    Source {
        pool: ClientPool::new(1, TransportLimits::DEFAULT),
        tls,
        routing,
        created: AtomicUsize::new(0),
    }
}

#[derive(Debug, Error)]
#[error("the test config refuses the environment")]
struct Refused;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum HeaderKind {
    #[default]
    Text,
    Numeric,
    BadName,
    BadValue,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum UrlKind {
    #[default]
    Good,
    Broken,
}

#[derive(Debug, Default)]
struct TestConfig {
    fail: bool,
    header: HeaderKind,
    url: UrlKind,
}

impl BaseRealtimeHttpConfig for TestConfig {
    type Error = Refused;

    fn get_api_base(&self, api_base: Option<&str>) -> Result<String, Refused> {
        Ok(api_base.unwrap_or_default().to_owned())
    }

    fn get_api_key(&self, api_key: Option<&str>) -> Result<String, Refused> {
        Ok(api_key.unwrap_or_default().to_owned())
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
        api_version: Option<&str>,
    ) -> String {
        let base = match self.url {
            UrlKind::Good => api_base.unwrap_or_default(),
            UrlKind::Broken => "not-a-url",
        };
        format!("{base}/custom/{model}/{}", api_version.unwrap_or("none"))
    }

    fn get_transcription_session_url(&self, url: &RealtimeUrl<'_>) -> String {
        format!(
            "{}/transcription/{}/{}",
            url.api_base.unwrap_or_default(),
            url.model,
            url.api_version.unwrap_or("none")
        )
    }

    fn validate_environment(&self, _environment: &Environment<'_>) -> Result<HeaderMap, Refused> {
        if self.fail {
            return Err(Refused);
        }
        let value = match self.header {
            HeaderKind::Text | HeaderKind::BadName => Json::Text(String::from("configured")),
            HeaderKind::Numeric => Json::Number(Number::from(7_u8)),
            HeaderKind::BadValue => Json::Text(String::from("bad\nvalue")),
        };
        let name = match self.header {
            HeaderKind::BadName => String::from("bad name"),
            HeaderKind::Text | HeaderKind::Numeric | HeaderKind::BadValue => {
                String::from("api-key")
            }
        };
        Ok(HeaderMap::from([(name, value)]))
    }
}

struct Request {
    head: String,
    body: Vec<u8>,
}

struct Reply {
    status: u16,
    headers: &'static str,
    body: &'static [u8],
}

fn ok_reply() -> Reply {
    Reply {
        status: 200,
        headers: "",
        body: b"{}",
    }
}

fn runtime() -> Runtime {
    let builder = Builder::new_current_thread().enable_all();
    builder.build().unwrap()
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default(),
    )
    .unwrap()
}

fn logging_env(redactor: &Redactor) -> PrePostEnv<'_> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &SINK,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    }
}

fn json_object(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = serde_json::from_str::<Json>(text).unwrap() else {
        panic!("the fixture is a JSON object");
    };
    fields
}

fn session_request() -> IndexMap<String, Json> {
    json_object(r#"{"session":{"model":"m"}}"#)
}

fn alloy_session() -> IndexMap<String, Json> {
    json_object(r#"{"voice":"alloy"}"#)
}

fn extra_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect()
}

// A pre_call replaces the logging state's initial null input; before it, nothing touched the field
fn input_untouched(state: &LoggingState) -> bool {
    state.model_call_details.get("input") == Some(&Json::Null)
}

fn input_recorded(state: &LoggingState, expected: &IndexMap<String, Json>) -> bool {
    state.model_call_details.get("input") == Some(&Json::Object(expected.clone()))
}

fn content_length(head: &str) -> usize {
    head.to_ascii_lowercase()
        .lines()
        .find_map(|line| line.strip_prefix("content-length: ")?.trim().parse().ok())
        .unwrap_or(0)
}

async fn read_request(stream: &mut TcpStream) -> Request {
    let marker = b"\r\n\r\n";
    let mut raw = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0, "the client closed before its request ended");
        raw.extend_from_slice(&chunk[..count]);
        let Some(at) = raw
            .windows(marker.len())
            .position(|window| window == marker)
        else {
            continue;
        };
        let head = String::from_utf8(raw[..at].to_vec()).unwrap();
        let body = raw[at + marker.len()..].to_vec();
        if body.len() >= content_length(&head) {
            return Request { head, body };
        }
    }
}

async fn serve(std_listener: StdListener, replies: &[Reply]) -> Vec<Request> {
    let listener = TcpListener::from_std(std_listener).unwrap();
    let mut requests = Vec::new();
    for reply in replies {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        let mut head = format!(
            "HTTP/1.1 {} X\r\ncontent-length: {}\r\n",
            reply.status,
            reply.body.len()
        );
        if !reply.headers.is_empty() {
            head.push_str(reply.headers);
            head.push_str("\r\n");
        }
        head.push_str("connection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(reply.body).await.unwrap();
        requests.push(request);
    }
    requests
}

fn exchange<T>(replies: &[Reply], run: impl FnOnce(&str) -> T) -> (String, Vec<Request>, T) {
    let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let address = std_listener.local_addr().unwrap();
    let api_base = format!("http://{address}");
    let output = thread::scope(|scope| {
        let server = scope.spawn(move || runtime().block_on(serve(std_listener, replies)));
        let run_output = run(&api_base);
        (server.join().unwrap(), run_output)
    });
    (api_base, output.0, output.1)
}

// A server that accepts and never answers, so the client's own deadline is what ends the send
fn held<T>(run: impl FnOnce(&str) -> T) -> T {
    let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let address = std_listener.local_addr().unwrap();
    thread::scope(|scope| {
        let server = scope.spawn(move || {
            runtime().block_on(async move {
                let listener = TcpListener::from_std(std_listener).unwrap();
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut chunk = [0_u8; 1024];
                let _read = stream.read(&mut chunk).await;
                sleep(Duration::from_millis(150)).await;
            })
        });
        let output = run(&format!("http://{address}"));
        server.join().unwrap();
        output
    })
}

fn session_call<'call>(
    endpoint: SessionEndpoint,
    api_base: &'call str,
    request_data: &'call IndexMap<String, Json>,
) -> SessionCall<'call> {
    SessionCall {
        endpoint,
        api_base,
        api_key: "sk-long-lived",
        request_data,
        model: None,
        extra_headers: None,
        api_version: None,
        timeout: None,
    }
}

fn fallback_headers(head: &str) -> bool {
    let lower = head.to_ascii_lowercase();
    lower.contains("authorization: bearer sk-long-lived")
        && lower.contains("content-type: application/json")
}

fn assert_client_secrets_wire(
    answer: &UpstreamResponse,
    request: &Request,
    data: &IndexMap<String, Json>,
) {
    assert_eq!(answer.status, 201);
    assert_eq!(answer.body, b"not json");
    assert_eq!(
        answer.headers.get("x-test").unwrap().to_str().unwrap(),
        "kept"
    );
    assert!(
        request
            .head
            .starts_with("POST /v1/v1/realtime/client_secrets HTTP/1.1")
    );
    assert!(fallback_headers(&request.head));
    assert_eq!(
        serde_json::from_slice::<Json>(&request.body).unwrap(),
        Json::Object(data.clone())
    );
}

fn assert_fallback_args(
    state: &LoggingState,
    url: &str,
    complete: Option<&IndexMap<String, Json>>,
) {
    let Some(Json::Object(args)) = state.model_call_details.get("additional_args") else {
        panic!("the pre_call keeps its arguments");
    };
    assert_eq!(args.get("api_base"), Some(&Json::Text(url.to_owned())));
    assert_eq!(
        args.get("complete_input_dict"),
        complete.map(|data| Json::Object(data.clone())).as_ref()
    );
    assert!(
        matches!(args.get("headers"), Some(Json::Object(headers)) if headers.contains_key("Authorization"))
    );
}

fn assert_sdp_multipart_wire(answer: &UpstreamResponse, request: &Request) {
    assert_eq!(answer.status, 200);
    let head = &request.head;
    assert!(head.starts_with("POST /v1/v1/realtime/calls HTTP/1.1"));
    assert!(
        head.to_ascii_lowercase()
            .contains("authorization: bearer ep-test")
    );
    assert!(head.contains(&format!("boundary={BOUNDARY}")));
    assert_eq!(request.body, expected_multipart());
}

fn assert_calls_recorded(state: &LoggingState, base: &str) {
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&Json::Text(String::from("realtime_sdp_offer")))
    );
    let Some(Json::Object(args)) = state.model_call_details.get("additional_args") else {
        panic!("the pre_call keeps its arguments");
    };
    assert_eq!(
        args.get("api_base"),
        Some(&Json::Text(format!("{base}/v1/realtime/calls")))
    );
    assert_eq!(
        args.get("session"),
        Some(&Json::Object(json_object(
            r#"{"voice":"alloy","type":"realtime","model":"m"}"#
        )))
    );
    assert!(!args.contains_key("complete_input_dict"));
}

#[test]
fn session_posts_to_the_client_secrets_fallback() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let replies = [Reply {
        status: 201,
        headers: "content-type: application/json\r\nx-test: kept",
        body: b"not json",
    }];
    let (_, requests, answers) = exchange(&replies, |base| {
        let trimmed = format!("{base}/v1/");
        runtime().block_on(async {
            let deps = TransportDeps {
                config: None,
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let call = session_call(SessionEndpoint::ClientSecrets, &trimmed, &data);
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    assert_client_secrets_wire(&answers.unwrap(), &requests[0], &data);
    assert!(input_recorded(&state, &data));
}

#[test]
fn session_posts_to_the_transcription_fallback() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let (api_base, requests, answers) = exchange(&[ok_reply()], |base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: None,
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let call = session_call(SessionEndpoint::TranscriptionSessions, base, &data);
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    assert_eq!(answers.unwrap().status, 200);
    assert!(
        requests[0]
            .head
            .starts_with("POST /v1/realtime/transcription_sessions HTTP/1.1")
    );
    assert!(fallback_headers(&requests[0].head));
    assert!(!state.model_call_details.contains_key("api_key"));
    assert_fallback_args(
        &state,
        &format!("{api_base}/v1/realtime/transcription_sessions"),
        Some(&data),
    );
}

#[test]
fn session_config_names_the_url_headers_and_extra_headers_win() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let config = TestConfig::default();
    let data = session_request();
    let extra = extra_headers(&[("api-key", "override"), ("x-test", "test")]);
    let (_, requests, answers) = exchange(&[ok_reply()], |api_base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: Some(&config),
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let call = SessionCall {
                endpoint: SessionEndpoint::ClientSecrets,
                api_base,
                api_key: "sk-long-lived",
                request_data: &data,
                model: None,
                extra_headers: Some(&extra),
                api_version: Some("v9"),
                timeout: None,
            };
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    assert_eq!(answers.unwrap().status, 200);
    assert!(requests[0].head.starts_with("POST /custom//v9 HTTP/1.1"));
    let lower = requests[0].head.to_ascii_lowercase();
    assert!(lower.contains("api-key: override"));
    assert!(lower.contains("x-test: test"));
    assert!(!lower.contains("api-key: configured"));
}

#[test]
fn calls_send_the_sdp_as_unnamed_multipart_parts() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let session = alloy_session();
    let replies = [Reply {
        status: 200,
        headers: "content-type: application/sdp",
        body: b"v=0\r\n",
    }];
    let (_, requests, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: None,
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let call = CallsCall {
                api_base,
                ephemeral_key: "ep-test",
                sdp: b"v=0\r\n",
                model: Some("m"),
                session: Some(&session),
                extra_headers: None,
                api_version: None,
                timeout: None,
            };
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            super::calls::calls_post(&deps, &call, &mut log).await
        })
    });
    assert_sdp_multipart_wire(&answers.unwrap(), &requests[0]);
}

#[test]
fn calls_record_the_offer_the_url_and_the_session() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let session = alloy_session();
    let (api_base, _, answers) = exchange(&[ok_reply()], |api_base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: None,
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let call = CallsCall {
                api_base,
                ephemeral_key: "ep-test",
                sdp: b"v=0\r\n",
                model: Some("m"),
                session: Some(&session),
                extra_headers: None,
                api_version: None,
                timeout: None,
            };
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            super::calls::calls_post(&deps, &call, &mut log).await
        })
    });
    assert_eq!(answers.unwrap().status, 200);
    assert_calls_recorded(&state, &api_base);
}

fn expected_multipart() -> Vec<u8> {
    let session = b"{\"voice\":\"alloy\",\"type\":\"realtime\",\"model\":\"m\"}";
    let mut body = Vec::new();
    for (name, content_type, data) in [
        ("sdp", "text/plain", b"v=0\r\n".to_vec()),
        ("session", "application/json", session.to_vec()),
    ] {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes(),
        );
        body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

#[rstest]
#[case(None, Some("m"), r#"{"type":"realtime","model":"m"}"#)]
#[case(Some("{}"), Some("m"), r#"{"type":"realtime","model":"m"}"#)]
#[case(
    Some(r#"{"type":"transcription"}"#),
    Some("m"),
    r#"{"type":"transcription","model":"m"}"#
)]
#[case(
    Some(r#"{"model":null}"#),
    Some("m"),
    r#"{"type":"realtime","model":null}"#
)]
#[case(
    Some(r#"{"type":"transcription","model":"x"}"#),
    None,
    r#"{"type":"transcription","model":"x"}"#
)]
#[case(Some("{}"), Some(""), r#"{"type":"realtime","model":""}"#)]
fn call_session_defaults_fill_only_absent_keys(
    #[case] session: Option<&str>,
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    let supplied = session.map(json_object);
    assert_eq!(
        session_data(supplied.as_ref(), model),
        json_object(expected)
    );
}

#[test]
fn calls_extra_headers_override_the_ephemeral_authorization() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let extra = BTreeMap::from([(
        String::from("Authorization"),
        String::from("Bearer caller-token"),
    )]);
    let replies = [ok_reply()];
    let (_, requests, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: None,
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let call = CallsCall {
                api_base,
                ephemeral_key: "ep-test",
                sdp: b"v=0\r\n",
                model: None,
                session: None,
                extra_headers: Some(&extra),
                api_version: None,
                timeout: None,
            };
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            super::calls::calls_post(&deps, &call, &mut log).await
        })
    });
    assert_eq!(answers.unwrap().status, 200);
    let lower = requests[0].head.to_ascii_lowercase();
    assert!(lower.contains("authorization: bearer caller-token"));
    assert!(!lower.contains("authorization: bearer ep-test"));
}

#[test]
fn invalid_or_oversized_sdp_fails_before_the_pre_call() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let answers = runtime().block_on(async {
        let deps = TransportDeps {
            config: None,
            source: &source,
            supplied: None,
            limits: TINY,
        };
        let mut answers = Vec::new();
        for sdp in [b"\xff\xfe".as_slice(), b"v=0\r\n".as_slice()] {
            let call = CallsCall {
                api_base: "http://127.0.0.1:9",
                ephemeral_key: "ep-test",
                sdp,
                model: None,
                session: None,
                extra_headers: None,
                api_version: None,
                timeout: None,
            };
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            answers.push(super::calls::calls_post(&deps, &call, &mut log).await);
        }
        answers
    });
    assert!(matches!(answers[0], Err(RealtimeHttpError::Sdp(_))));
    assert!(matches!(answers[1], Err(RealtimeHttpError::Sdp(_))));
    assert!(input_untouched(&state));
    assert!(!state.model_call_details.contains_key("additional_args"));
}

#[test]
fn client_selection_reuses_an_async_client_and_replaces_a_sync_one() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let supplied = source.client().unwrap();
    let data = session_request();
    let replies = vec![ok_reply(), ok_reply(), ok_reply()];
    let (_, _, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let mut answers = Vec::new();
            for kind in [Some(ClientKind::Asynchronous), Some(ClientKind::Sync), None] {
                let supplied_client = kind.map(|kind| SuppliedClient {
                    kind,
                    transport: &supplied,
                });
                let deps = TransportDeps {
                    config: None,
                    source: &source,
                    supplied: supplied_client,
                    limits: ApiLimits::default(),
                };
                let call = session_call(SessionEndpoint::ClientSecrets, api_base, &data);
                let mut log = LogContext {
                    state: &mut state,
                    env: &envelope,
                };
                answers.push(session_post(&deps, &call, &mut log).await);
            }
            answers
        })
    });
    assert!(
        answers
            .iter()
            .all(|answer| answer.as_ref().unwrap().status == 200)
    );
    assert_eq!(source.created.load(Ordering::SeqCst), 3);
    assert_eq!(source.pool.client_count(), 1);
}

#[test]
fn the_supplied_timeout_reaches_the_deadline() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let answer = held(|api_base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: None,
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let mut call = session_call(SessionEndpoint::ClientSecrets, api_base, &data);
            call.timeout = Some(Duration::from_millis(50));
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    let Err(RealtimeHttpError::Timeout(failure)) = answer else {
        panic!("the deadline ends the send");
    };
    assert!(failure.message.contains("Timeout passed=0.05"));
}

#[test]
fn a_status_without_a_config_is_the_masked_response() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let replies = [Reply {
        status: 429,
        headers: "content-type: text/plain\r\nretry-after: 2\r\ncontent-encoding: gzip",
        body: b"slow",
    }];
    let (_, _, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: None,
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let call = session_call(SessionEndpoint::ClientSecrets, api_base, &data);
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    let Err(RealtimeHttpError::Status(failure)) = answers else {
        panic!("a 429 is the masked status error");
    };
    assert_masked_status(&failure);
}

// The masked response: the status and body survive, content-encoding is dropped and a fresh content-length replaces it
fn assert_masked_status(failure: &StatusFailure) {
    assert_eq!(failure.status_code, 429);
    assert_eq!(failure.message, "slow");
    assert_eq!(
        failure.headers.get("retry-after"),
        Some(&Json::Text(String::from("2")))
    );
    assert_eq!(
        failure.headers.get("content-length"),
        Some(&Json::Text(String::from("4")))
    );
    assert!(!failure.headers.contains_key("content-encoding"));
}

#[test]
fn a_status_with_a_config_is_the_provider_error_class() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let config = TestConfig::default();
    let data = session_request();
    let replies = [Reply {
        status: 429,
        headers: "content-type: text/plain\r\nretry-after: 2",
        body: b"slow",
    }];
    let (_, _, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: Some(&config),
                source: &source,
                supplied: None,
                limits: ApiLimits::default(),
            };
            let call = session_call(SessionEndpoint::ClientSecrets, api_base, &data);
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    let Err(RealtimeHttpError::Provider(error)) = answers else {
        panic!("a 429 enters the provider error class");
    };
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow");
    assert!(!error.status_code_is_synthesized);
    assert_eq!(
        error.headers.get("retry-after"),
        Some(&Json::Text(String::from("2")))
    );
}

#[test]
fn a_config_error_happens_before_the_pre_call_and_the_send() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let refusing = TestConfig {
        fail: true,
        ..TestConfig::default()
    };
    let answer = runtime().block_on(async {
        let deps = TransportDeps {
            config: Some(&refusing),
            source: &source,
            supplied: None,
            limits: ApiLimits::default(),
        };
        let call = session_call(SessionEndpoint::ClientSecrets, "http://127.0.0.1:9", &data);
        let mut log = LogContext {
            state: &mut state,
            env: &envelope,
        };
        session_post(&deps, &call, &mut log).await
    });
    assert!(matches!(answer, Err(RealtimeHttpError::Config(_))));
    assert!(input_untouched(&state));
    assert_eq!(source.created.load(Ordering::SeqCst), 1);
}

#[test]
fn a_non_text_config_header_fails_the_wire_build() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let numeric = TestConfig {
        header: HeaderKind::Numeric,
        ..TestConfig::default()
    };
    let answer = runtime().block_on(async {
        let deps = TransportDeps {
            config: Some(&numeric),
            source: &source,
            supplied: None,
            limits: ApiLimits::default(),
        };
        let call = session_call(SessionEndpoint::ClientSecrets, "http://127.0.0.1:9", &data);
        let mut log = LogContext {
            state: &mut state,
            env: &envelope,
        };
        session_post(&deps, &call, &mut log).await
    });
    assert!(matches!(
        answer,
        Err(RealtimeHttpError::HeaderNotText { .. })
    ));
    // the typed failure is betterlite's own: the pre_call has already run, and no request left the process
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&Json::Object(data))
    );
}

#[test]
fn a_pre_call_failure_stops_the_send() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    state.request_logging.raw_request_only = true;
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let answer = runtime().block_on(async {
        let deps = TransportDeps {
            config: None,
            source: &source,
            supplied: None,
            limits: ApiLimits::default(),
        };
        let call = session_call(SessionEndpoint::ClientSecrets, "http://127.0.0.1:9", &data);
        let mut log = LogContext {
            state: &mut state,
            env: &envelope,
        };
        session_post(&deps, &call, &mut log).await
    });
    assert!(matches!(answer, Err(RealtimeHttpError::Logging(_))));
}

fn corpus() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/realtime_sdp")
}

#[test]
fn replays_corpus() {
    let tiny = ApiLimits {
        max_json_bytes: 32,
        max_event_bytes: 32,
        max_items: 32,
    };
    let mut seen = 0_usize;
    for entry in fs::read_dir(corpus()).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        seen += 1;
        let bytes = fs::read(&path).unwrap();
        let _whole = decode_realtime_sdp(&bytes, ApiLimits::default());
        let _capped = decode_realtime_sdp(&bytes, tiny);
    }
    assert!(seen > 0_usize);
}

#[test]
fn the_decoder_reads_utf8_up_to_the_json_body_cap() {
    let limits = ApiLimits {
        max_json_bytes: 4,
        max_event_bytes: 1,
        max_items: 1,
    };
    assert_eq!(decode_realtime_sdp(b"v=0", limits).unwrap(), "v=0");
    assert!(matches!(
        decode_realtime_sdp(b"v=0\r\n", limits),
        Err(super::decode::SdpError::Limit(_))
    ));
    assert!(matches!(
        decode_realtime_sdp(b"\xff", limits),
        Err(super::decode::SdpError::Utf8(_))
    ));
}

#[test]
fn the_corpus_holds_an_invalid_utf8_offer() {
    let bytes = fs::read(corpus().join("invalid-utf8")).unwrap();
    assert!(decode_realtime_sdp(&bytes, ApiLimits::default()).is_err());
}
