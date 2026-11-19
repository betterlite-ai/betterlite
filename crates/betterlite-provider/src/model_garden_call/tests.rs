#![cfg(test)]

use std::collections::BTreeMap;
use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ChatRequest};
use betterlite_openai::compatible_line_stream::Mode;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_vertex::model_policy::error::VertexAiError;
use chrono::DateTime;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;
use uuid::Uuid;

use crate::compatible_chat_call::send::CompatibleChatOutcome;

use super::{ModelGardenCall, completion};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
const REPLY: &str = r#"{"id":"chat-1","object":"chat.completion","created":1,"model":"served-model","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const LINES: &str =
    "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"index\":0}]}\ndata: [DONE]\n";

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn logging(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    (state, env)
}

fn messages() -> Vec<ChatMessage> {
    serde_json::from_str(r#"[{"role": "user", "content": "hi"}]"#).unwrap()
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

fn origin(listener: &TcpListener) -> String {
    format!("http://{}", listener.local_addr().unwrap())
}

async fn serve(listener: &TcpListener, status: u16, reply: &[u8]) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        let text = String::from_utf8(request.clone()).unwrap();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let length = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|line| line.strip_prefix("content-length: ")?.parse::<usize>().ok())
                .unwrap_or(0);
            if body.len() >= length {
                break;
            }
        }
    }
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        reply.len()
    );
    let mut response = head.into_bytes();
    response.extend_from_slice(reply);
    stream.write_all(&response).await.unwrap();
    String::from_utf8(request).unwrap()
}

struct Fixture {
    model: String,
    messages: Vec<ChatMessage>,
    options: IndexMap<String, Json>,
    api_base: Option<String>,
    location: Option<String>,
    project: Option<String>,
    resolved_project: String,
    token: SecretString,
    timeout: Option<Duration>,
}

impl Fixture {
    fn new(model: &str) -> Self {
        Self {
            model: model.to_owned(),
            messages: messages(),
            options: IndexMap::new(),
            api_base: None,
            location: None,
            project: Some(String::from("proj")),
            resolved_project: String::from("auth-proj"),
            token: SecretString::from("sk-token"),
            timeout: None,
        }
    }

    fn call(&self) -> ModelGardenCall<'_> {
        ModelGardenCall {
            model: &self.model,
            messages: &self.messages,
            optional_params: &self.options,
            api_base: self.api_base.as_deref(),
            vertex_location: self.location.as_deref(),
            vertex_project: self.project.as_deref(),
            resolved_project: &self.resolved_project,
            access_token: &self.token,
            timeout: self.timeout,
            mode: Mode::Async,
            identifier: fixed_id,
            clock: || 1_700_000_000,
        }
    }
}

fn answer<'call>(
    call: &'call ModelGardenCall<'_>,
    listener: &'call TcpListener,
    status: u16,
    reply: &'call [u8],
) -> BoxFuture<'call, (String, Result<CompatibleChatOutcome, VertexAiError>)> {
    Box::pin(async move {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
        let (mut state, env) = logging(&redactor);
        timeout(EXCHANGE_TIMEOUT, async {
            tokio::join!(serve(listener, status, reply), async {
                completion(call, &client(), &mut state, &env).await
            })
        })
        .await
        .unwrap()
    })
}

#[tokio::test]
async fn garden_call_appends_one_chat_path_to_a_publisher_target() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut fixture = Fixture::new("meta/llama-3.3-70b-instruct-maas");
    // the fixture server stands in for the OpenAPI base the leaf derives from the model and location
    fixture.api_base = Some(format!(
        "{}/v1/projects/proj/locations/us-central1/endpoints/openapi",
        origin(&listener)
    ));
    let (request, result) = answer(&fixture.call(), &listener, 200, REPLY.as_bytes()).await;
    let CompatibleChatOutcome::Reply(reply) = result.unwrap() else {
        panic!("a buffered reply");
    };
    assert_eq!(reply.response.model, "vertex_ai/served-model");
    assert!(request.starts_with(
        "POST /v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions HTTP/1.1"
    ));
    let (_, payload) = request.split_once("\r\n\r\n").unwrap();
    let body: ChatRequest = serde_json::from_str(payload).unwrap();
    assert_eq!(body.messages, messages());
    assert_eq!(request.matches("/chat/completions").count(), 1);
    assert!(request.contains(r#""model": "meta/llama-3.3-70b-instruct-maas""#));
    assert!(request.contains(r#""stream": false"#));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-token")
    );
}

#[tokio::test]
async fn garden_call_sends_an_endpoint_id_with_an_empty_body_model() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut fixture = Fixture::new("openai/5464397967697903616");
    fixture.api_base = Some(format!(
        "{}/v1beta1/projects/proj/locations/us-central1/endpoints/5464397967697903616",
        origin(&listener)
    ));
    let (request, result) = answer(&fixture.call(), &listener, 200, REPLY.as_bytes()).await;
    assert!(matches!(result.unwrap(), CompatibleChatOutcome::Reply(_)));
    assert!(request.starts_with(
        "POST /v1beta1/projects/proj/locations/us-central1/endpoints/5464397967697903616/chat/completions HTTP/1.1"
    ));
    assert!(request.contains(r#""model": """#));
}

#[tokio::test]
async fn garden_call_keeps_an_explicit_base_without_a_colon_verb() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut fixture = Fixture::new("openai/5464397967697903616");
    fixture.api_base = Some(format!("{}/dedicated", origin(&listener)));
    fixture.location = Some(String::from("not a location"));
    let (request, result) = answer(&fixture.call(), &listener, 200, REPLY.as_bytes()).await;
    assert!(matches!(result.unwrap(), CompatibleChatOutcome::Reply(_)));
    assert!(request.starts_with("POST /dedicated/chat/completions HTTP/1.1"));
    assert_eq!(request.matches("/chat/completions").count(), 1);
    assert!(!request.contains(":predict"));
}

#[tokio::test]
async fn garden_call_wraps_a_status_error_in_a_vertex_500() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let failure = br#"{"error":{"message":"denied"}}"#;
    let mut fixture = Fixture::new("meta/llama-3");
    fixture.api_base = Some(origin(&listener));
    let (_, result) = answer(&fixture.call(), &listener, 400, failure).await;
    let Err(error) = result else {
        panic!("a vertex error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, r#"{"error":{"message":"denied"}}"#);
}

#[tokio::test]
async fn garden_call_wraps_a_target_error_in_a_vertex_500() {
    let mut fixture = Fixture::new("123");
    fixture.location = Some(String::from("not a location"));
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let result = completion(&fixture.call(), &client(), &mut state, &env).await;
    let Err(error) = result else {
        panic!("a vertex error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "invalid vertex_location format");
}

#[tokio::test]
async fn garden_call_wraps_a_wrong_stream_type_in_a_vertex_500() {
    let mut fixture = Fixture::new("123");
    fixture
        .options
        .insert("stream".to_owned(), Json::Text(String::from("yes")));
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let result = completion(&fixture.call(), &client(), &mut state, &env).await;
    let Err(error) = result else {
        panic!("a vertex error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "the stream option must be a boolean");
}

#[tokio::test]
async fn garden_call_keeps_an_explicit_stream_true() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut fixture = Fixture::new("meta/llama-3");
    fixture.api_base = Some(origin(&listener));
    fixture
        .options
        .insert("stream".to_owned(), Json::Bool(true));
    let (request, result) = answer(&fixture.call(), &listener, 200, LINES.as_bytes()).await;
    assert!(matches!(result.unwrap(), CompatibleChatOutcome::Stream(_)));
    assert!(request.contains(r#""stream": true"#));
}

#[tokio::test]
async fn garden_call_normalizes_a_null_stream_to_false() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut fixture = Fixture::new("meta/llama-3");
    fixture.api_base = Some(origin(&listener));
    fixture.options.insert("stream".to_owned(), Json::Null);
    let (request, result) = answer(&fixture.call(), &listener, 200, REPLY.as_bytes()).await;
    assert!(matches!(result.unwrap(), CompatibleChatOutcome::Reply(_)));
    assert!(request.contains(r#""stream": false"#));
}

#[tokio::test]
async fn garden_call_bounds_the_request_with_the_caller_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut fixture = Fixture::new("meta/llama-3");
    fixture.api_base = Some(origin(&listener));
    fixture.timeout = Some(Duration::from_millis(20));
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    // the fixture accepts the connection and never answers, so only the caller's deadline ends the call
    let mut holder = JoinSet::new();
    holder.spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        pending::<()>().await;
    });
    let result = timeout(
        EXCHANGE_TIMEOUT,
        completion(&fixture.call(), &client(), &mut state, &env),
    )
    .await
    .unwrap();
    holder.abort_all();
    let Err(error) = result else {
        panic!("a vertex error");
    };
    assert_eq!(error.status_code, 500);
}
