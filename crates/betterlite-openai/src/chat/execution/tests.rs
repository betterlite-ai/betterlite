#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::{MockPermission, MockPlan};
use betterlite_config::sections::mock_response::{MockResponse, UsageDefaults};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tracing::subscriber::set_default;
use uuid::Uuid;

use super::ChatExecutor;
use crate::chat::deployment::{ChatDeployment, OpenAiDefaults};
use crate::chat::response::{ChatResponse, ChatUsage};
use crate::chat::{ChatRequest, MockSettings};
use crate::embedding::EmbeddingResponse;
use crate::openai_base_config::CatalogCapabilities;

const ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":"no"},"finish_reason":"stop"}]}"#;

// the stub reports the injected id and the effective mock content through the response id
fn echo_mock(
    plan: &MockPlan<'_>,
    _usage: UsageDefaults,
    model: &str,
    id: &str,
    created: u64,
) -> Result<ChatResponse, MockError> {
    let Some(MockResponse::Text(content)) = plan.response else {
        return Err(MockError::WrongType {
            field: "mock_response",
        });
    };
    let failure = match content.as_str() {
        "timeout" => Some(MockError::Timeout),
        "rate-limit" => Some(MockError::RateLimit),
        _ => None,
    };
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(ChatResponse {
        id: format!("{id}={content}"),
        object: String::from("chat.completion"),
        created,
        model: model.to_owned(),
        choices: Vec::new(),
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    })
}

fn unused_embedding(
    _response: Option<&MockResponse>,
    _model: &str,
) -> Result<EmbeddingResponse, MockError> {
    Err(MockError::EmbeddingFailure)
}

fn fixed_clock() -> u64 {
    1_700_000_000
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

fn executor(
    entries: &str,
    seed: u64,
    operator_mock: Option<MockResponse>,
    permission: MockPermission,
) -> ChatExecutor {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let config = loader.load(&format!("model_list: {entries}\n")).unwrap();
    let defaults = OpenAiDefaults {
        request_timeout: None,
        api_bases: Vec::new(),
        api_keys: Vec::new(),
        organization: Some("org-7".to_owned()),
    };
    let deployments: Vec<ChatDeployment> = config
        .model_list
        .iter()
        .map(|deployment| ChatDeployment::resolve(deployment, &defaults).unwrap())
        .collect();
    let responses = operator_mock.map_or_else(BTreeMap::new, |response| {
        deployments
            .iter()
            .map(|deployment| (deployment.model_id.clone(), response.clone()))
            .collect()
    });
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ChatExecutor {
        client: ProxiedClient::outbound(&policy).unwrap(),
        deployments,
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(seed)),
        clock: fixed_clock,
        next_id: fixed_id,
        mock: MockSettings {
            responses,
            usage: UsageDefaults::default(),
            permission,
        },
        mock_chat: echo_mock,
        mock_embedding: unused_embedding,
    }
}

async fn provider(listener: TcpListener, status: u16, body: &str, delay: Duration) -> String {
    let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("no request reached the provider")
        .unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut sent = vec![0_u8; length];
    stream.read_exact(&mut sent).await.unwrap();
    sleep(delay).await;
    let reply = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\nx-request-id: r1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _written = stream.write_all(reply.as_bytes()).await;
    format!("{head}{}", String::from_utf8(sent).unwrap())
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    (listener, base)
}

#[tokio::test]
async fn sends_the_mapped_body_and_answers_with_the_requested_model() {
    let (listener, base) = listen().await;
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k}}}}]"
    );
    let chat = executor(&entries, 1, None, MockPermission::default());
    let body = br#"{"model":"g","stream":false,"temperature":1,"user":"u","vendor_flag":{"a":1},"messages":[{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{}"}}]}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    let deployment = chat.deployments.first().unwrap();
    let (sent, answered) = tokio::join!(
        provider(listener, 200, ANSWER, Duration::ZERO),
        chat.send(deployment, &request)
    );
    assert!(sent.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    assert!(sent.contains("authorization: Bearer k"));
    assert!(sent.contains("openai-organization: org-7"));
    assert!(sent.contains("content-type: application/json"));
    assert!(sent.contains("accept: application/json"));
    let upstream: Value = serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let expected = r#"{"model":"up","temperature":1,"vendor_flag":{"a":1},"messages":[{"role":"assistant","tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{}"}}]}]}"#;
    assert_eq!(upstream, serde_json::from_str::<Value>(expected).unwrap());
    let outcome = answered.unwrap();
    assert_eq!(outcome.upstream.status, 200);
    assert_eq!(outcome.upstream.headers.get("x-request-id").unwrap(), "r1");
    let client_body = serde_json::to_value(&outcome.response).unwrap();
    let client_expected = r#"{"id":"c1","object":"chat.completion","created":7,"model":"g","usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0},"choices":[{"index":0,"finish_reason":"stop","provider_specific_fields":{},"message":{"role":"assistant","content":null,"provider_specific_fields":{"refusal":"no"}}}]}"#;
    assert_eq!(
        client_body,
        serde_json::from_str::<Value>(client_expected).unwrap()
    );
}

#[rstest]
#[case::provider_error(400, ANSWER, Duration::ZERO, "status 400")]
#[case::not_a_completion(200, "[]", Duration::ZERO, "not a chat completion")]
#[case::slower_than_the_timeout(
    200,
    ANSWER,
    Duration::from_secs(2),
    "within the deployment timeout"
)]
#[tokio::test]
async fn reports_what_went_wrong_upstream(
    #[case] status: u16,
    #[case] answer: &str,
    #[case] delay: Duration,
    #[case] message: &str,
) {
    let (listener, base) = listen().await;
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k, timeout: 0.3}}}}]"
    );
    let chat = executor(&entries, 1, None, MockPermission::default());
    let request = ChatRequest::parse(br#"{"messages":[]}"#).unwrap();
    let deployment = chat.deployments.first().unwrap();
    let (_sent, answered) = tokio::join!(
        provider(listener, status, answer, delay),
        chat.send(deployment, &request)
    );
    let error = answered.err().unwrap();
    assert!(error.to_string().contains(message), "{error}");
    if delay > Duration::ZERO {
        assert_eq!(error.failure().status, 408);
    }
}

#[rstest]
#[case::streaming(
    "{model: openai/m, api_key: k}",
    r#"{"stream":true,"messages":[]}"#,
    "streaming"
)]
#[case::other_provider("{model: azure/m, api_key: k}", r#"{"messages":[]}"#, "azure/m")]
#[case::no_api_key("{model: openai/m}", r#"{"messages":[]}"#, "no API key")]
#[case::unreachable(
    "{model: openai/m, api_key: k, api_base: 'http://127.0.0.1:1'}",
    r#"{"messages":[]}"#,
    "could not be reached"
)]
#[tokio::test]
async fn refuses_calls_it_cannot_make(
    #[case] params: &str,
    #[case] body: &str,
    #[case] message: &str,
) {
    let chat = executor(
        &format!("[{{model_name: g, litellm_params: {params}}}]"),
        1,
        None,
        MockPermission::default(),
    );
    let request = ChatRequest::parse(body.as_bytes()).unwrap();
    let error = chat
        .send(chat.deployments.first().unwrap(), &request)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains(message), "{error}");
}

#[rstest]
#[case::default(r#"{"messages":[],"stream":true}"#, Some(true))]
#[case::enabled(
    r#"{"messages":[],"stream":true,"stream_options":{"include_usage":true}}"#,
    Some(true)
)]
#[case::disabled(
    r#"{"messages":[],"stream":true,"stream_options":{"include_usage":false}}"#,
    Some(false)
)]
#[case::buffered(r#"{"messages":[]}"#, None)]
fn stream_options_preserve_caller(#[case] input: &str, #[case] usage: Option<bool>) {
    let chat = executor("[]", 1, None, MockPermission::default());
    let request = ChatRequest::parse(input.as_bytes()).unwrap();
    let body = chat.body(&request, "up").unwrap();
    assert_eq!(
        body.get("stream"),
        request.stream.then_some(&Value::Bool(true))
    );
    let actual = body
        .get("stream_options")
        .and_then(|options| options.get("include_usage"))
        .and_then(Value::as_bool);
    assert_eq!(actual, usage);
}

#[rstest]
#[case::success(200)]
#[case::provider_failure(429)]
#[tokio::test]
async fn stream_distinguishes_status(#[case] status: u16) {
    let (listener, base) = listen().await;
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k}}}}]"
    );
    let chat = executor(&entries, 1, None, MockPermission::default());
    let request = ChatRequest::parse(br#"{"model":"g","messages":[],"stream":true}"#).unwrap();
    let body = r#"{"error":{"type":"requests","param":"p"}}"#;
    let (sent, result) = tokio::join!(
        provider(listener, status, body, Duration::ZERO),
        chat.stream(chat.deployments.first().unwrap(), &request)
    );
    assert!(sent.contains("accept: text/event-stream"));
    if status == 200 {
        let response = result
            .unwrap()
            .buffered(chat.client.limits.body_bytes)
            .await
            .unwrap();
        assert_eq!(response.status, status);
        assert_eq!(response.body, body.as_bytes());
    } else {
        let failure = result.err().unwrap().failure();
        assert_eq!(failure.status, status);
        assert_eq!(failure.error_type, "throttling_error");
        assert_eq!(failure.param.as_deref(), Some("p"));
    }
}

async fn staged_provider(listener: TcpListener, go: Arc<Notify>) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let first = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n8\r\ndata: 1\n\r\n";
    stream.write_all(first.as_bytes()).await.unwrap();
    go.notified().await;
    stream
        .write_all(b"8\r\ndata: 2\n\r\n0\r\n\r\n")
        .await
        .unwrap();
}

struct Wire {
    status: u16,
    answer: &'static str,
    delay: Duration,
}

const OK: Wire = Wire {
    status: 200,
    answer: ANSWER,
    delay: Duration::ZERO,
};

const SMALL_BODY: TransportLimits = TransportLimits {
    body_bytes: 10,
    ..TransportLimits::DEFAULT
};

const SMALL_HEADERS: TransportLimits = TransportLimits {
    header_bytes: 10,
    ..TransportLimits::DEFAULT
};

#[rstest]
#[case::answer_within_the_limits(false, false, OK, TransportLimits::DEFAULT, "ok")]
#[case::success_body_over_the_limit(false, false, OK, SMALL_BODY, "larger than betterlite accepts")]
#[case::error_body_over_the_limit(false, false, Wire { status: 500, ..OK }, SMALL_BODY, "larger than betterlite accepts")]
#[case::headers_over_the_limit(false, false, OK, SMALL_HEADERS, "larger than betterlite accepts")]
#[case::the_deployment_client_applies_its_limits(
    false,
    true,
    OK,
    SMALL_BODY,
    "larger than betterlite accepts"
)]
#[case::deployment_deadline_covers_the_answer(false, false, Wire { delay: Duration::from_secs(2), ..OK }, TransportLimits::DEFAULT, "within the deployment timeout")]
#[case::stream_reads_chunk_by_chunk(true, false, OK, TransportLimits::DEFAULT, "data: 1|data: 2")]
#[tokio::test]
async fn http_pool_chat_regression_behavior(
    #[case] stream: bool,
    #[case] own_client: bool,
    #[case] wire: Wire,
    #[case] limits: TransportLimits,
    #[case] expected: &str,
) {
    let (listener, base) = listen().await;
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k, timeout: 0.3}}}}]"
    );
    let mut chat = executor(&entries, 1, None, MockPermission::default());
    if own_client {
        let model_id = chat.deployments.first().unwrap().model_id.clone();
        let sibling = ProxiedClient {
            limits,
            ..chat.client.clone()
        };
        chat.client.deployments = Arc::new(BTreeMap::from([(model_id, sibling)]));
    } else {
        chat.client.limits = limits;
    }
    let deployment = chat.deployments.first().unwrap();
    let observed = if stream {
        let go = Arc::new(Notify::new());
        let mut server = JoinSet::new();
        server.spawn(staged_provider(listener, Arc::clone(&go)));
        let request = ChatRequest::parse(br#"{"messages":[],"stream":true}"#).unwrap();
        let mut response = chat.stream(deployment, &request).await.unwrap();
        let mut chunks = Vec::new();
        while let Some(chunk) = response.response.chunk().await.unwrap() {
            chunks.push(String::from_utf8(chunk.to_vec()).unwrap().trim().to_owned());
            go.notify_one();
        }
        chunks.join("|")
    } else {
        let request = ChatRequest::parse(br#"{"messages":[]}"#).unwrap();
        let (_sent, answered) = tokio::join!(
            provider(listener, wire.status, wire.answer, wire.delay),
            chat.send(deployment, &request)
        );
        answered.map_or_else(|error| error.to_string(), |_outcome| "ok".to_owned())
    };
    assert!(observed.contains(expected), "{observed}");
}

const UPSTREAM_ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":"upstream"},"finish_reason":"stop"}]}"#;

#[derive(Clone, Copy)]
enum Expected {
    Content(&'static str),
    Failure(u16, &'static str),
    Upstream,
}

fn allowed() -> MockPermission {
    MockPermission {
        allow_client_mock_response: true,
    }
}

#[rstest]
#[case::operator_mock_answers_without_a_provider(
    Some("operator"),
    None,
    MockPermission::default(),
    Expected::Content("operator")
)]
#[case::an_allowed_client_mock_answers(
    None,
    Some("client"),
    allowed(),
    Expected::Content("client")
)]
#[case::an_allowed_client_mock_beats_the_operator_one(
    Some("operator"),
    Some("client"),
    allowed(),
    Expected::Content("client")
)]
#[case::a_denied_client_mock_is_stripped(
    None,
    Some("client"),
    MockPermission::default(),
    Expected::Upstream
)]
#[case::the_rate_limit_sentinel_keeps_litellm_status(
    Some("rate-limit"),
    None,
    MockPermission::default(),
    Expected::Failure(429, "throttling_error")
)]
#[case::the_timeout_sentinel_keeps_litellm_status(
    Some("timeout"),
    None,
    MockPermission::default(),
    Expected::Failure(408, "invalid_request_error")
)]
#[tokio::test]
async fn mock_calls_behavior_tests_behavior(
    #[case] operator: Option<&'static str>,
    #[case] client: Option<&'static str>,
    #[case] permission: MockPermission,
    #[case] expected: Expected,
) {
    let (listener, base) = listen().await;
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k, timeout: 0.1}}}}]"
    );
    let operator_mock = operator.map(|text| MockResponse::Text(text.to_owned()));
    let chat = executor(&entries, 1, operator_mock, permission);
    let request_body = client.map_or_else(
        || String::from(r#"{"model":"g","messages":[]}"#),
        |text| format!(r#"{{"model":"g","messages":[],"mock_response":"{text}"}}"#),
    );
    let request = ChatRequest::parse(request_body.as_bytes()).unwrap();
    let deployment = chat.deployments.first().unwrap();
    match expected {
        Expected::Content(text) => {
            let outcome = chat.send(deployment, &request).await.unwrap();
            let expected_id = format!("chatcmpl-{}={text}", fixed_id());
            assert_eq!(outcome.response.id, expected_id);
            assert_eq!(outcome.response.created, fixed_clock());
            assert_eq!(outcome.upstream.status, 200);
        }
        Expected::Failure(status, error_type) => {
            let error = chat.send(deployment, &request).await.err().unwrap();
            let failure = error.failure();
            assert_eq!(failure.status, status);
            assert_eq!(failure.error_type, error_type);
        }
        Expected::Upstream => {
            let (sent, answered) = tokio::join!(
                provider(listener, 200, UPSTREAM_ANSWER, Duration::ZERO),
                chat.send(deployment, &request)
            );
            drop(answered.unwrap());
            assert!(!sent.contains("mock_response"), "{sent}");
            assert!(!sent.contains("mock_tool_calls"), "{sent}");
        }
    }
}

#[tokio::test]
async fn a_denied_client_tool_call_is_stripped_before_its_shape_is_used() {
    let (listener, base) = listen().await;
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k}}}}]"
    );
    let chat = executor(&entries, 1, None, MockPermission::default());
    let request = ChatRequest::parse(
        br#"{"model":"g","messages":[],"mock_tool_calls":[{"function":{"name":"f","arguments":{"a":1}}}]}"#,
    )
    .unwrap();
    let deployment = chat.deployments.first().unwrap();
    let (sent, answered) = tokio::join!(
        provider(listener, 200, UPSTREAM_ANSWER, Duration::ZERO),
        chat.send(deployment, &request)
    );
    drop(answered.unwrap());
    assert!(!sent.contains("mock_tool_calls"), "{sent}");
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_stripped_client_mock_warns_with_the_field_name() {
    let entries = "[{model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, timeout: 0.1}}]";
    let chat = executor(entries, 1, None, MockPermission::default());
    let request = ChatRequest::parse(
        br#"{"model":"g","messages":[],"mock_response":"client","mock_tool_calls":[]}"#,
    )
    .unwrap();
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    let guard = set_default(subscriber);
    drop(chat.send(chat.deployments.first().unwrap(), &request).await);
    drop(guard);
    let warnings = captured.text();
    assert!(warnings.contains("mock_response"), "{warnings}");
    assert!(warnings.contains("mock_tool_calls"), "{warnings}");
}

#[derive(Clone, Copy)]
enum Base {
    Plain,
    Userinfo(&'static str),
    PercentEncoded,
    Ftp,
}

fn authorization(sent: &str) -> String {
    sent.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(": ")?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.to_owned())
        })
        .unwrap_or_else(|| "-".to_owned())
}

const CALL: &str = r#"{"model":"g","messages":[]}"#;
const HIJACK: &str = r#"{"model":"http://127.0.0.1:1/v1","messages":[],"endpoint":"http://127.0.0.1:1/v1","url":"http://127.0.0.1:1/v1","destination":"http://127.0.0.1:1/v1"}"#;

async fn boundary_observation(base: Base, api_key: Option<&str>, body: &str) -> String {
    let (listener, plain) = listen().await;
    let api_base = match base {
        Base::Plain => plain,
        Base::Userinfo(userinfo) => plain.replace("http://", &format!("http://{userinfo}")),
        Base::PercentEncoded => plain.replace("http://", "http://us%40er:p%40ss@"),
        Base::Ftp => plain.replace("http://", "ftp://"),
    };
    let key = api_key.map_or_else(String::new, |key| format!(", api_key: {key}"));
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{api_base}'{key}}}}}]"
    );
    let chat = executor(&entries, 1, None, MockPermission::default());
    let request = ChatRequest::parse(body.as_bytes()).unwrap();
    let deployment = chat.deployments.first().unwrap();
    if matches!(base, Base::Ftp) {
        return chat
            .send(deployment, &request)
            .await
            .map_or_else(|error| error.to_string(), |_outcome| "answered".to_owned());
    }
    let mut provider_task = JoinSet::new();
    provider_task.spawn(provider(listener, 200, ANSWER, Duration::ZERO));
    if let Err(error) = chat.send(deployment, &request).await {
        drop(provider_task);
        return error.to_string();
    }
    let sent = provider_task.join_next().await.unwrap().unwrap();
    let upstream = sent.split_once("\r\n\r\n").map_or("", |(_, text)| text);
    format!(
        "{}|auth={}|forwarded={}",
        sent.lines().next().unwrap_or_default(),
        authorization(&sent),
        upstream.contains("\"destination\"")
    )
}

async fn redirecting(listener: TcpListener, location: String) {
    let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("no request reached the redirecting provider")
        .unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut sent = vec![0_u8; length];
    stream.read_exact(&mut sent).await.unwrap();
    let reply = format!(
        "HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    );
    let _written = stream.write_all(reply.as_bytes()).await;
}

async fn redirect_observation() -> String {
    let (source_listener, source_base) = listen().await;
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_base = format!("http://{}/v1", target_listener.local_addr().unwrap());
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{source_base}', api_key: k}}}}]"
    );
    let chat = executor(&entries, 1, None, MockPermission::default());
    let request = ChatRequest::parse(CALL.as_bytes()).unwrap();
    let deployment = chat.deployments.first().unwrap();
    let mut target_task = JoinSet::new();
    target_task.spawn(provider(target_listener, 200, ANSWER, Duration::ZERO));
    let redirect = redirecting(source_listener, format!("{target_base}/chat/completions"));
    let (answered, _redirected) = tokio::join!(chat.send(deployment, &request), redirect);
    let sent = target_task.join_next().await.unwrap().unwrap();
    format!(
        "ok={}|auth={}|{}",
        answered.is_ok(),
        authorization(&sent),
        sent.lines().next().unwrap_or_default()
    )
}

#[derive(Clone, Copy)]
enum Scenario {
    Wire(Base, Option<&'static str>, &'static str),
    Redirect,
}

async fn observation(scenario: Scenario) -> String {
    match scenario {
        Scenario::Wire(base, api_key, body) => boundary_observation(base, api_key, body).await,
        Scenario::Redirect => redirect_observation().await,
    }
}

#[rstest]
#[case::userinfo_becomes_basic_auth_over_the_api_key(
    Scenario::Wire(Base::Userinfo("user:pass@"), Some("k"), CALL),
    "POST /v1/chat/completions HTTP/1.1|auth=Basic dXNlcjpwYXNz|forwarded=false"
)]
#[case::percent_encoded_userinfo_is_decoded(
    Scenario::Wire(Base::PercentEncoded, Some("k"), CALL),
    "POST /v1/chat/completions HTTP/1.1|auth=Basic dXNAZXI6cEBzcw==|forwarded=false"
)]
#[case::userinfo_alone_authenticates_a_loopback_provider(
    Scenario::Wire(Base::Userinfo("user:pass@"), None, CALL),
    "POST /v1/chat/completions HTTP/1.1|auth=Basic dXNlcjpwYXNz|forwarded=false"
)]
#[case::the_api_key_bearer_is_used_without_userinfo(
    Scenario::Wire(Base::Plain, Some("k"), CALL),
    "POST /v1/chat/completions HTTP/1.1|auth=Bearer k|forwarded=false"
)]
#[case::request_fields_cannot_change_the_destination(
    Scenario::Wire(Base::Plain, Some("k"), HIJACK),
    "POST /v1/chat/completions HTTP/1.1|auth=Bearer k|forwarded=true"
)]
#[case::no_credentials_is_an_error(
    Scenario::Wire(Base::Plain, None, CALL),
    "no API key is set for this deployment, in litellm_settings or in OPENAI_API_KEY"
)]
#[case::an_ftp_base_is_refused_by_key(
    Scenario::Wire(Base::Ftp, Some("k"), CALL),
    "the provider could not be reached: the configured api_base cannot be used: litellm_params.api_base uses the ftp scheme, and only http and https are supported"
)]
#[case::a_provider_redirect_does_not_forward_credentials(
    Scenario::Redirect,
    "ok=true|auth=-|GET /v1/chat/completions HTTP/1.1"
)]
#[tokio::test]
async fn http_boundaries_url_boundary_tests_behavior(
    #[case] scenario: Scenario,
    #[case] expected: &str,
) {
    assert_eq!(observation(scenario).await, expected);
}
