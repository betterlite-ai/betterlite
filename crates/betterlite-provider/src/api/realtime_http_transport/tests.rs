#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener as StdListener;
use std::path::{Path, PathBuf};
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
use betterlite_http::proxy::transport_error::TransportError;
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
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};
use tokio::time::sleep;

use super::TransportDeps;
use super::client::{RealtimeClientSource, SuppliedClient};
use super::decode::decode_realtime_sdp;
use super::error::{RealtimeHttpError, StatusFailure, timeout_failure};
use super::session::{SessionCall, SessionEndpoint, session_post};

mod calls;
mod wire;

const INIT: &str = r#"{"model":"realtime","call_type":"realtime","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
// sha256 over the two multipart parts in order: name, bytes, content type
const BOUNDARY: &str = "2f5cc9be1c163676e4ae2f347a1309d402542a6a431366f1f96b68a587ba2451";

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
    source_with(TransportLimits::DEFAULT)
}

fn source_with(limits: TransportLimits) -> Source {
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("model_list: []")
    .unwrap();
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    let tls = TlsPolicy::resolve(&config, &environment, None).unwrap();
    Source {
        pool: ClientPool::new(1, limits),
        tls,
        routing,
        created: AtomicUsize::new(0),
    }
}

// A transport with no provider config: the handlers take their fallback paths, and C names the config they would take
const fn fallback_deps<'deps>(
    source: &'deps Source,
    supplied: Option<SuppliedClient<'deps>>,
    limits: ApiLimits,
) -> TransportDeps<'deps, TestConfig> {
    TransportDeps {
        config: None,
        source,
        supplied,
        limits,
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
    hold: bool,
}

const OK_REPLY: Reply = Reply {
    status: 200,
    headers: "",
    body: b"{}",
    hold: false,
};

// A server that reads the request and never answers, so the client's own deadline is what ends the send
const HELD_REPLY: Reply = Reply {
    status: 200,
    headers: "",
    body: b"",
    hold: true,
};

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(INIT).unwrap();
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        init,
        setup,
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .unwrap()
}

fn logging_env(redactor: &Redactor) -> PrePostEnv<'_> {
    let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap();
    let decision = Decision {
        content: false,
        key_info: false,
    };
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision,
        now,
        redactor,
        log: &SINK,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
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
        let body_start = at.checked_add(marker.len()).unwrap();
        let body = raw[body_start..].to_vec();
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
        requests.push(request);
        if reply.hold {
            sleep(Duration::from_millis(150)).await;
            continue;
        }
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
    }
    requests
}

fn exchange<T, F>(replies: &[Reply], run: F) -> (String, Vec<Request>, T)
where
    F: FnOnce(&str) -> T,
{
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

const fn session_call<'call>(
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
        hold: false,
    }];
    let (_, requests, answers) = exchange(&replies, |base| {
        let trimmed = format!("{base}/v1/");
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
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
    let (api_base, requests, answers) = exchange(&[OK_REPLY], |base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
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
    let (_, requests, answers) = exchange(&[OK_REPLY], |api_base| {
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

// The supplied client is fetched once (created=1), then an async one is reused and a sync or absent one reaches the pool again
#[rstest]
#[case::async_client(Some(ClientKind::Asynchronous), 1)]
#[case::sync_client(Some(ClientKind::Sync), 2)]
#[case::no_client(None, 2)]
fn client_selection_reuses_an_async_client_and_replaces_a_sync_one(
    #[case] kind: Option<ClientKind>,
    #[case] created: usize,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let supplied = source.client().unwrap();
    let data = session_request();
    let (_, _, answers) = exchange(&[OK_REPLY], |api_base| {
        runtime().block_on(async {
            let supplied_client = kind.map(|selected| SuppliedClient {
                kind: selected,
                transport: &supplied,
            });
            let deps = fallback_deps(&source, supplied_client, ApiLimits::default());
            let call = session_call(SessionEndpoint::ClientSecrets, api_base, &data);
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    assert_eq!(answers.unwrap().status, 200);
    assert_eq!(source.created.load(Ordering::SeqCst), created);
    assert_eq!(source.pool.client_count(), 1);
}

#[test]
fn the_supplied_timeout_reaches_the_deadline() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let (_, _, answer) = exchange(&[HELD_REPLY], |api_base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
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

// The handler's own timeout text: a whole-second timeout prints as its integer, a fractional one as typed
#[test]
fn a_whole_second_timeout_prints_as_an_integer() {
    let whole = timeout_failure(Duration::from_secs(1), Duration::from_millis(1500));
    assert_eq!(
        whole.message,
        "litellm.Timeout: Connection timed out. Timeout passed=1, time taken=1.500 seconds"
    );
    let fractional = timeout_failure(Duration::from_millis(1500), Duration::from_millis(1500));
    assert_eq!(
        fractional.message,
        "litellm.Timeout: Connection timed out. Timeout passed=1.5, time taken=1.500 seconds"
    );
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
        hold: false,
    }];
    let (_, _, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
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

// 400 is the boundary the status check treats as an error, not a success
#[test]
fn a_bad_request_status_is_rejected() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let replies = [Reply {
        status: 400,
        headers: "content-type: text/plain",
        body: b"bad",
        hold: false,
    }];
    let (_, _, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
            let call = session_call(SessionEndpoint::ClientSecrets, api_base, &data);
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    let Err(RealtimeHttpError::Status(failure)) = answers else {
        panic!("a 400 is the masked status error");
    };
    assert_eq!(failure.status_code, 400);
    assert_eq!(failure.message, "bad");
}

// The masked status carries bytes it cannot read as text by their count, never as replacement characters
#[test]
fn a_masked_reply_body_that_is_not_utf8_is_counted() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let data = session_request();
    let replies = [Reply {
        status: 429,
        headers: "content-type: application/octet-stream",
        body: b"\xff\xfe",
        hold: false,
    }];
    let (_, _, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
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
    assert_eq!(failure.message, "2 bytes that are not UTF-8");
    assert_eq!(
        failure.headers.get("content-length"),
        Some(&Json::Text(String::from("2")))
    );
}

// The transport's body cap: a reply past the limit is a transport error, never a silently cut answer
#[test]
fn a_reply_body_over_the_transport_limit_is_a_transport_error() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = source_with(TransportLimits {
        body_bytes: 4,
        ..TransportLimits::DEFAULT
    });
    let data = session_request();
    let replies = [Reply {
        status: 200,
        headers: "content-type: text/plain",
        body: b"12345",
        hold: false,
    }];
    let (_, _, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
            let call = session_call(SessionEndpoint::ClientSecrets, api_base, &data);
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            session_post(&deps, &call, &mut log).await
        })
    });
    let Err(RealtimeHttpError::Transport(TransportError::BodyTooLarge { limit })) = answers else {
        panic!("a reply past the body cap is a transport error");
    };
    assert_eq!(limit, 4);
}

#[test]
fn a_transport_failure_with_a_config_is_a_synthesized_provider_error() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let config = TestConfig::default();
    let data = session_request();
    let answer = runtime().block_on(async {
        let deps = TransportDeps {
            config: Some(&config),
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
    let Err(RealtimeHttpError::Provider(error)) = answer else {
        panic!("a refused connection enters the provider error class");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, "the connection could not be made");
}

#[test]
fn a_timeout_with_a_config_is_a_provider_timeout() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let config = TestConfig::default();
    let data = session_request();
    let (_, _, answer) = exchange(&[HELD_REPLY], |api_base| {
        runtime().block_on(async {
            let deps = TransportDeps {
                config: Some(&config),
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
    let Err(RealtimeHttpError::Provider(error)) = answer else {
        panic!("a timeout enters the provider error class");
    };
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert!(error.message.contains("Timeout passed=0.05"));
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
        hold: false,
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
        header: HeaderKind::Text,
        url: UrlKind::Good,
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
        fail: false,
        header: HeaderKind::Numeric,
        url: UrlKind::Good,
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
        let deps = fallback_deps(&source, None, ApiLimits::default());
        let call = session_call(SessionEndpoint::ClientSecrets, "http://127.0.0.1:9", &data);
        let mut log = LogContext {
            state: &mut state,
            env: &envelope,
        };
        session_post(&deps, &call, &mut log).await
    });
    assert!(matches!(answer, Err(RealtimeHttpError::Logging(_))));
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/realtime_sdp")
}

#[test]
fn replays_corpus() {
    let tiny = ApiLimits {
        max_json_bytes: 32,
        max_event_bytes: 32,
        max_items: 32,
    };
    let replayed = fs::read_dir(corpus())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .map(|path| {
            let bytes = fs::read(path).unwrap();
            let _whole = decode_realtime_sdp(&bytes, ApiLimits::default());
            let _capped = decode_realtime_sdp(&bytes, tiny);
        })
        .count();
    assert!(replayed > 0_usize);
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
