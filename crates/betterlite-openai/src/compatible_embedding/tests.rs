#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
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
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};

use super::error::EmbeddingError;
use super::{
    EmbeddingCall, EmbeddingPlan, Mode, finish_embedding, plan_embedding, request_body,
    send_embedding,
};
use crate::compatible_wire::error::OpenAiLikeError;
use crate::openai_base_config::Headers;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"embedding","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const EMBEDDING: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25]}],"model":"m","usage":{"prompt_tokens":1,"total_tokens":1}}"#;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

struct Sink(Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0.lock().unwrap().push(record);
    }
}

fn observe<T, A>(action: A) -> (T, LoggingState, Vec<LogRecord>)
where
    A: FnOnce(&mut LoggingState, &PrePostEnv<'_>) -> T,
{
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "trace".to_owned(),
        Duration::default,
    )
    .unwrap();
    let sink = Sink(Mutex::new(Vec::new()));
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    let result = action(&mut state, &env);
    (result, state, sink.0.into_inner().unwrap())
}

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn options(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn call(api_base: Option<&str>, explicit: Option<Headers>, custom_endpoint: bool) -> EmbeddingCall {
    EmbeddingCall {
        model: "m".to_owned(),
        input: Json::List(vec![Json::Text("a".to_owned())]),
        api_key: Some(SecretString::from("k".to_owned())),
        api_base: api_base.map(str::to_owned),
        options: options(r#"{"dimensions":2,"user":""}"#),
        headers: explicit,
        custom_endpoint,
        timeout: Duration::from_secs(30),
        mode: Mode::Async,
    }
}

fn plan(url: &str, deadline: Duration, mode: Mode) -> EmbeddingPlan {
    EmbeddingPlan {
        url: url.to_owned(),
        headers: Headers::from([
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), "Bearer k".to_owned()),
        ]),
        data: request_body(
            "m".to_owned(),
            &Json::List(vec![Json::Text("a".to_owned())]),
            IndexMap::new(),
        ),
        input: Json::List(vec![Json::Text("a".to_owned())]),
        timeout: deadline,
        mode,
    }
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

async fn read_request(stream: &mut TcpStream) -> String {
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
                return text;
            }
        }
    }
}

// the status and body to send back, and how long to wait first
async fn serve(listener: &TcpListener, status: u16, body: &[u8], pause: Duration) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let request = read_request(&mut stream).await;
    sleep(pause).await;
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    if stream.write_all(head.as_bytes()).await.is_ok() {
        stream.write_all(body).await.unwrap_or_default();
    }
    request
}

async fn exchange(
    status: u16,
    body: &[u8],
    deadline: Duration,
    pause: Duration,
) -> (Result<Json, EmbeddingError>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/embeddings", listener.local_addr().unwrap());
    let client = client();
    let plan = plan(&url, deadline, Mode::Async);
    let sent = send_embedding(&client, &plan);
    timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(sent, serve(&listener, status, body, pause))
    })
    .await
    .unwrap()
}

#[rstest]
#[case::kept_and_overriding(
    r#"{"dimensions":0,"flag":false,"tag":"x","empty":[],"none":null,"blank":""}"#,
    r#"{"model":"m","input":["a"],"dimensions":0,"flag":false,"tag":"x","empty":[]}"#
)]
#[case::overriding_model_and_input(
    r#"{"model":"other","input":["b"]}"#,
    r#"{"model":"other","input":["b"]}"#
)]
#[case::empty_overrides_are_left_out(
    r#"{"model":null,"input":""}"#,
    r#"{"model":"m","input":["a"]}"#
)]
fn options_set_to_none_or_empty_text_are_left_out(#[case] extra: &str, #[case] expected: &str) {
    let body = request_body(
        "m".to_owned(),
        &Json::List(vec![Json::Text("a".to_owned())]),
        options(extra),
    );
    assert_eq!(serde_json::to_value(&body).unwrap(), parsed(expected));
}

#[test]
fn planning_validates_filters_and_logs_the_request() {
    let (plan, state, _) = observe(|state, env| {
        plan_embedding(call(Some("https://h/v1"), None, false), state, env).unwrap()
    });
    assert_eq!(plan.url, "https://h/v1/embeddings");
    assert_eq!(plan.timeout, Duration::from_secs(30));
    assert_eq!(plan.mode, Mode::Async);
    assert_eq!(
        plan.headers,
        Headers::from([
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), "Bearer k".to_owned()),
        ])
    );
    let expected = parsed(r#"{"model":"m","input":["a"],"dimensions":2}"#);
    assert_eq!(serde_json::to_value(&plan.data).unwrap(), expected);
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    assert_eq!(details["input"], parsed(r#"["a"]"#));
    assert_eq!(
        details["additional_args"]["api_base"],
        "https://h/v1/embeddings"
    );
    assert_eq!(details["additional_args"]["complete_input_dict"], expected);
}

#[test]
fn the_plan_debug_names_the_authorization_header_without_its_value() {
    let planned = plan("https://h/embeddings", Duration::from_secs(1), Mode::Async);
    let shown = format!("{planned:?}");
    assert!(shown.contains("Authorization"));
    assert!(!shown.contains("Bearer k"));
}

#[test]
fn explicit_headers_are_kept_and_a_custom_endpoint_is_used_as_given() {
    let explicit = Headers::from([("X-Team".to_owned(), "1".to_owned())]);
    let (plan, _, _) = observe(|state, env| {
        plan_embedding(
            call(Some("https://h/own"), Some(explicit), true),
            state,
            env,
        )
        .unwrap()
    });
    assert_eq!(plan.url, "https://h/own");
    assert_eq!(
        plan.headers,
        Headers::from([
            ("X-Team".to_owned(), "1".to_owned()),
            ("Authorization".to_owned(), "Bearer k".to_owned()),
        ])
    );
}

// LiteLLM sends such a body without a Content-Type, and the shared transport always names the JSON body
#[tokio::test]
async fn the_transport_adds_a_json_content_type_the_plan_did_not_carry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let explicit = Headers::from([("X-Team".to_owned(), "1".to_owned())]);
    let (plan, _, _) = observe(|state, env| {
        plan_embedding(call(Some(&url), Some(explicit), true), state, env).unwrap()
    });
    let client = client();
    let (sent, request) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(
            send_embedding(&client, &plan),
            serve(&listener, 200, EMBEDDING.as_bytes(), Duration::ZERO)
        )
    })
    .await
    .unwrap();
    sent.unwrap();
    let lowered = request.to_ascii_lowercase();
    assert!(!plan.headers.contains_key("Content-Type"));
    assert!(lowered.contains("content-type: application/json"));
    assert!(lowered.contains("x-team: 1"));
}

#[test]
fn a_missing_base_stops_planning_before_anything_is_logged() {
    let (error, state, records) =
        observe(|state, env| plan_embedding(call(None, None, false), state, env).unwrap_err());
    assert!(matches!(
        error,
        EmbeddingError::Upstream(OpenAiLikeError {
            status_code: 400,
            ..
        })
    ));
    assert_eq!(error.status_code(), Some(400));
    assert!(!state.model_call_details.contains_key("additional_args"));
    assert!(records.is_empty());
}

#[tokio::test]
async fn a_successful_send_posts_the_body_with_the_planned_headers() {
    let (sent, request) = exchange(
        200,
        EMBEDDING.as_bytes(),
        Duration::from_secs(5),
        Duration::ZERO,
    )
    .await;
    assert_eq!(
        serde_json::to_value(sent.unwrap()).unwrap(),
        parsed(EMBEDDING)
    );
    let lowered = request.to_ascii_lowercase();
    assert!(request.starts_with("POST /embeddings HTTP/1.1"));
    assert!(lowered.contains("authorization: bearer k"));
    assert!(lowered.contains("content-type: application/json"));
    assert!(request.ends_with(r#"{"model":"m","input":["a"]}"#));
}

#[rstest]
#[case::bad_request(400, b"no", 400, "no")]
#[case::not_found(404, b"", 404, "")]
#[case::server_error(500, b"boom", 500, "boom")]
#[tokio::test]
async fn an_error_answer_keeps_its_status_and_text(
    #[case] status: u16,
    #[case] body: &[u8],
    #[case] expected: u16,
    #[case] text: &str,
) {
    let (sent, _) = exchange(status, body, Duration::from_secs(5), Duration::ZERO).await;
    let error = sent.unwrap_err();
    assert_eq!(error.status_code(), Some(expected));
    assert_eq!(error.to_string(), text);
}

#[tokio::test]
async fn an_error_body_that_is_not_utf8_is_its_own_error() {
    let (sent, _) = exchange(502, &[0xff, 0xfe], Duration::from_secs(5), Duration::ZERO).await;
    let error = sent.unwrap_err();
    assert!(matches!(
        error,
        EmbeddingError::StatusBody { status: 502, .. }
    ));
    assert_eq!(error.status_code(), Some(502));
}

#[tokio::test]
async fn a_reply_that_is_not_json_is_a_500() {
    let (sent, _) = exchange(200, b"nope", Duration::from_secs(5), Duration::ZERO).await;
    let error = sent.unwrap_err();
    assert!(matches!(error, EmbeddingError::Reply(_)));
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn a_slow_answer_is_a_408_timeout() {
    let (sent, _) = exchange(
        200,
        EMBEDDING.as_bytes(),
        Duration::from_millis(100),
        Duration::from_secs(1),
    )
    .await;
    let error = sent.unwrap_err();
    assert!(matches!(error, EmbeddingError::Timeout(_)));
    assert_eq!(error.status_code(), Some(408));
    assert_eq!(error.to_string(), "Timeout error occurred.");
}

#[tokio::test]
async fn a_refused_connection_is_a_500() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/embeddings", listener.local_addr().unwrap());
    drop(listener);
    let error = send_embedding(&client(), &plan(&url, Duration::from_secs(5), Mode::Async))
        .await
        .unwrap_err();
    assert!(matches!(error, EmbeddingError::Transport(_)));
    assert_eq!(error.status_code(), Some(500));
}

fn raw_responses(records: &[LogRecord]) -> usize {
    records
        .iter()
        .filter(|record| record.message.starts_with("RAW RESPONSE"))
        .count()
}

#[rstest]
#[case::sync(Mode::Sync)]
#[case::async_mode(Mode::Async)]
fn a_good_reply_is_logged_once_with_the_request_body(#[case] mode: Mode) {
    let reply = parsed(EMBEDDING);
    let (finished, state, records) = observe(|state, env| {
        let planned = plan("https://h/embeddings", Duration::from_secs(1), mode);
        finish_embedding(
            &planned,
            Ok(serde_json::from_value(reply).unwrap()),
            state,
            env,
        )
    });
    assert_eq!(finished.unwrap().model, "m");
    assert_eq!(raw_responses(&records), 1);
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    assert_eq!(
        parsed(details["original_response"].as_str().unwrap()),
        parsed(EMBEDDING)
    );
    assert_eq!(
        details["additional_args"]["complete_input_dict"],
        parsed(r#"{"model":"m","input":["a"]}"#)
    );
}

#[rstest]
#[case::sync_omits_the_outer_log(Mode::Sync, 0)]
#[case::async_logs_the_failure(Mode::Async, 1)]
fn a_failed_send_is_logged_only_in_async_mode(#[case] mode: Mode, #[case] logged: usize) {
    let (finished, state, records) = observe(|state, env| {
        let failed = EmbeddingError::Status {
            status: 500,
            text: "boom".to_owned(),
        };
        let planned = plan("https://h/embeddings", Duration::from_secs(1), mode);
        finish_embedding(&planned, Err(failed), state, env)
    });
    assert_eq!(finished.unwrap_err().status_code(), Some(500));
    assert_eq!(raw_responses(&records), logged);
    if logged == 1 {
        let details = serde_json::to_value(&state.model_call_details).unwrap();
        assert_eq!(details["original_response"], "boom");
        assert_eq!(details["additional_args"], parsed("{}"));
    }
}

#[rstest]
#[case::sync_keeps_the_success_log_only(Mode::Sync, 1)]
#[case::async_adds_the_failure_log(Mode::Async, 2)]
fn a_reply_that_cannot_be_built_is_logged_after_success_in_async_mode(
    #[case] mode: Mode,
    #[case] logged: usize,
) {
    let (finished, state, records) = observe(|state, env| {
        let planned = plan("https://h/embeddings", Duration::from_secs(1), mode);
        let reply = serde_json::from_str(r#"{"data":1}"#).unwrap();
        finish_embedding(&planned, Ok(reply), state, env)
    });
    let error = finished.unwrap_err();
    assert!(matches!(error, EmbeddingError::Response(_)));
    assert_eq!(error.status_code(), None);
    assert_eq!(raw_responses(&records), logged);
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    let shown = details["original_response"].as_str().unwrap();
    if mode == Mode::Async {
        assert_eq!(shown, error.to_string());
    } else {
        assert_eq!(parsed(shown), parsed(r#"{"data":1}"#));
    }
}
