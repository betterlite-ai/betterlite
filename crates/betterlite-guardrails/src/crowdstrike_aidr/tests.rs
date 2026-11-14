#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use axum::Router;
use axum::body::Bytes;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::StatusCode;
use secrecy::ExposeSecret as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::api::call_guard;
use super::error::CrowdStrikeError;
use super::handler::{ConstructorArgs, CrowdStrikeAidr, Dependencies};
use super::models::{EventType, GuardInput, GuardPayload, Message, MessageContent};
use crate::crowdstrike_streaming::StreamingParams;
use crate::generic_guardrail_contracts::GuardrailToolParam;
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::record::{GuardrailInvocationRecord, GuardrailSpanSink};

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn constructor(
    api_key: Option<&str>,
    api_base: Option<&str>,
    fail_on_error: Option<bool>,
) -> ConstructorArgs {
    ConstructorArgs {
        guardrail_name: Some("cs-aidr".to_owned()),
        api_key: api_key.map(str::to_owned),
        api_base: api_base.map(str::to_owned),
        fail_on_error,
        streaming: StreamingParams::default(),
        timeout: None,
    }
}

fn handler_for(api_base: &str, fail_on_error: Option<bool>) -> CrowdStrikeAidr {
    CrowdStrikeAidr::new(
        &constructor(Some("test-key"), Some(api_base), fail_on_error),
        &BTreeMap::<String, String>::new(),
        Dependencies {
            client: reqwest::Client::default(),
            sink: Box::new(DiscardSink),
            now: SystemTime::now,
        },
    )
    .expect("the constructor gets both secrets")
}

fn payload() -> GuardPayload {
    GuardPayload {
        guard_input: GuardInput {
            messages: vec![Message {
                role: "user".to_owned(),
                content: Some(MessageContent::Text("hi".to_owned())),
            }],
            tools: Some(Vec::<GuardrailToolParam>::new()),
        },
        event_type: EventType::Input,
        model: None,
        user_id: None,
        extra_info: None,
    }
}

async fn serve(
    status: StatusCode,
    body: &'static str,
) -> (String, Arc<Mutex<Vec<u8>>>, JoinSet<std::io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let answer = move |bytes: Bytes| {
        let sink = Arc::clone(&sink);
        async move {
            *sink.lock().unwrap() = bytes.to_vec();
            (status, body)
        }
    };
    let router = Router::new().route("/v1/guard_chat_completions", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, captured, server)
}

#[test]
fn constructor_validates_api_key() {
    let lookup = BTreeMap::from([("CS_AIDR_BASE_URL".to_owned(), "https://env.test".to_owned())]);
    let error = super::handler::Settings::resolve(&constructor(None, None, None), &lookup)
        .expect_err("the api key is missing");
    assert!(matches!(
        error,
        CrowdStrikeError::MissingEnvironment {
            name: "CS_AIDR_TOKEN"
        }
    ));
}

#[test]
fn constructor_validates_api_base() {
    let lookup = BTreeMap::from([("CS_AIDR_TOKEN".to_owned(), "env-key".to_owned())]);
    let error = super::handler::Settings::resolve(&constructor(None, None, None), &lookup)
        .expect_err("the api base is missing");
    assert!(matches!(
        error,
        CrowdStrikeError::MissingEnvironment {
            name: "CS_AIDR_BASE_URL"
        }
    ));
}

#[test]
fn constructor_reads_api_key_from_env() {
    let lookup = BTreeMap::from([
        ("CS_AIDR_TOKEN".to_owned(), "env-key".to_owned()),
        ("CS_AIDR_BASE_URL".to_owned(), "https://env.test".to_owned()),
    ]);
    let settings = super::handler::Settings::resolve(&constructor(None, None, None), &lookup)
        .expect("the environment provides both");
    assert_eq!(settings.api_key.expose_secret(), "env-key");
}

#[test]
fn constructor_reads_api_base_from_env() {
    let lookup = BTreeMap::from([
        ("CS_AIDR_TOKEN".to_owned(), "env-key".to_owned()),
        ("CS_AIDR_BASE_URL".to_owned(), "https://env.test".to_owned()),
    ]);
    let settings = super::handler::Settings::resolve(&constructor(None, None, None), &lookup)
        .expect("the environment provides both");
    assert_eq!(settings.api_base, "https://env.test");
}

#[test]
fn constructor_prefers_parameters_over_environment_and_rejects_empty_values() {
    let lookup = BTreeMap::from([
        ("CS_AIDR_TOKEN".to_owned(), "env-key".to_owned()),
        ("CS_AIDR_BASE_URL".to_owned(), "https://env.test".to_owned()),
    ]);
    let settings = super::handler::Settings::resolve(
        &constructor(Some("param-key"), Some("https://param.test"), None),
        &lookup,
    )
    .expect("the parameters win");
    assert_eq!(settings.api_key.expose_secret(), "param-key");
    assert_eq!(settings.api_base, "https://param.test");
    let empty = super::handler::Settings::resolve(&constructor(Some(""), Some(""), None), &lookup)
        .expect("an empty parameter falls back to the environment");
    assert_eq!(empty.api_key.expose_secret(), "env-key");
    assert_eq!(empty.api_base, "https://env.test");
}

#[test]
fn constructor_fail_on_error_defaults_true() {
    let lookup = BTreeMap::<String, String>::new();
    let unset = super::handler::Settings::resolve(
        &constructor(Some("key"), Some("https://cs.test"), None),
        &lookup,
    )
    .expect("both secrets are present");
    assert!(unset.fail_on_error);
    let disabled = super::handler::Settings::resolve(
        &constructor(Some("key"), Some("https://cs.test"), Some(false)),
        &lookup,
    )
    .expect("both secrets are present");
    assert!(!disabled.fail_on_error);
}

#[test]
fn constructor_initializes_streaming_defaults() {
    let lookup = BTreeMap::<String, String>::new();
    let mut args = constructor(Some("key"), Some("https://cs.test"), None);
    args.streaming = StreamingParams {
        buffer_until_moderated: Some(true),
        buffer_release_on_scan: None,
        end_of_stream_only: Some(true),
        sampling_rate: Some(12),
    };
    let settings = super::handler::Settings::resolve(&args, &lookup).expect("both secrets exist");
    assert!(settings.streaming.buffer_until_moderated);
    assert!(!settings.streaming.buffer_release_on_scan);
    assert!(settings.streaming.end_of_stream_only);
    assert_eq!(settings.streaming.sampling_rate, 12);
    let defaults = super::handler::Settings::resolve(
        &constructor(Some("key"), Some("https://cs.test"), None),
        &lookup,
    )
    .expect("both secrets exist");
    assert!(!defaults.streaming.buffer_until_moderated);
    assert_eq!(defaults.streaming.sampling_rate, 5);
}

#[test]
fn update_in_memory_litellm_params() {
    let mut guardrail = handler_for("https://cs.test", None);
    let params: LitellmParams = serde_json::from_str(
        r#"{
            "guardrail": "crowdstrike_aidr",
            "mode": "pre_call",
            "api_key": "new-key",
            "api_base": "https://new.test",
            "streaming_buffer_release_on_scan": true,
            "streaming_sampling_rate": 4
        }"#,
    )
    .expect("a well-formed litellm_params");
    guardrail
        .update_in_memory_params(&params)
        .expect("the streaming settings load");
    assert_eq!(guardrail.settings.api_key.expose_secret(), "new-key");
    assert_eq!(guardrail.settings.api_base, "https://new.test");
    assert!(guardrail.settings.streaming.buffer_release_on_scan);
    assert_eq!(guardrail.settings.streaming.sampling_rate, 4);
}

#[test]
fn get_supported_event_hooks() {
    let guardrail = handler_for("https://cs.test", None);
    assert_eq!(
        guardrail.get_supported_event_hooks(),
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall
        ])
    );
    assert_eq!(
        serde_json::to_string(&super::handler::SUPPORTED_EVENT_HOOKS).expect("hooks serialize"),
        r#"["pre_call","post_call"]"#
    );
}

#[test]
fn get_config_model() {
    let guardrail = handler_for("https://cs.test", None);
    let model = guardrail
        .get_config_model()
        .expect("the config model is known");
    let GuardrailValue::Object(fields) = model else {
        panic!("the config model is an object");
    };
    assert_eq!(
        fields.get("model"),
        Some(&GuardrailValue::Text("CrowdStrikeAidrConfig".to_owned()))
    );
    assert_eq!(
        fields.get("ui_friendly_name"),
        Some(&GuardrailValue::Text(
            "CrowdStrike AIDR Guardrail".to_owned()
        ))
    );
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_success() {
    let (base, _captured, _server) = serve(
        StatusCode::OK,
        r#"{"result": {"transformed": false, "detectors": {"p": 1}}}"#,
    )
    .await;
    let guardrail = handler_for(&base, None);
    let result = call_guard(&guardrail, &payload(), "test")
        .await
        .expect("a 200 with a parsable result succeeds");
    assert_eq!(result.transformed, Some(false));
    assert_eq!(
        result.detectors.as_ref().and_then(|d| d.get("p")),
        Some(&GuardrailValue::SignedInteger(1))
    );
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_blocked() {
    let (base, _captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"blocked": true}}"#).await;
    let guardrail = handler_for(&base, None);
    let error = call_guard(&guardrail, &payload(), "test")
        .await
        .expect_err("a blocked verdict raises");
    assert!(matches!(
        error,
        CrowdStrikeError::Blocked {
            guardrail_name: Some(ref name)
        } if name == "cs-aidr"
    ));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.detail().get("error"),
        Some(&GuardrailValue::Text(
            "Violated CrowdStrike AIDR guardrail policy".to_owned()
        ))
    );
    assert_eq!(
        error.detail().get("guardrail_name"),
        Some(&GuardrailValue::Text("cs-aidr".to_owned()))
    );
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_transformed_malformed() {
    let (base, _captured, _server) = serve(
        StatusCode::OK,
        r#"{"result": {"transformed": true, "guard_output": "nope"}}"#,
    )
    .await;
    let guardrail = handler_for(&base, None);
    let error = call_guard(&guardrail, &payload(), "test")
        .await
        .expect_err("an unparsable transformed reply fails closed");
    assert!(matches!(
        error,
        CrowdStrikeError::MalformedTransformed { .. }
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail().get("guardrail_name"),
        Some(&GuardrailValue::Null)
    );
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_malformed_not_transformed() {
    let (base, _captured, _server) = serve(
        StatusCode::OK,
        r#"{"result": {"transformed": false, "guard_output": "nope"}}"#,
    )
    .await;
    let guardrail = handler_for(&base, None);
    let error = call_guard(&guardrail, &payload(), "test")
        .await
        .expect_err("a reply that does not validate raises");
    assert!(matches!(error, CrowdStrikeError::ResponseJson { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_rejects_a_non_success_status() {
    let (base, _captured, _server) = serve(StatusCode::INTERNAL_SERVER_ERROR, "oops").await;
    let guardrail = handler_for(&base, None);
    let error = call_guard(&guardrail, &payload(), "test")
        .await
        .expect_err("a 500 raises");
    assert!(matches!(
        error,
        CrowdStrikeError::HttpStatus {
            status: StatusCode::INTERNAL_SERVER_ERROR
        }
    ));
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_sends_the_bearer_header() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let authorization = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&authorization);
    let answer = move |headers: axum::http::HeaderMap| {
        let sink = Arc::clone(&sink);
        async move {
            let value = headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            *sink.lock().unwrap() = value;
            (StatusCode::OK, r#"{"result": {}}"#)
        }
    };
    let router = Router::new().route("/v1/guard_chat_completions", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let guardrail = handler_for(&base, None);
    call_guard(&guardrail, &payload(), "test")
        .await
        .expect("the call succeeds");
    assert_eq!(*authorization.lock().unwrap(), "Bearer test-key");
}

const LIMIT: usize = 1024 * 1024;

// Answers the first request with the given raw bytes, then closes the connection
async fn serve_raw(reply: Vec<u8>) -> (String, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.read(&mut [0_u8; 8192]).await.unwrap();
        socket.write_all(&reply).await.unwrap();
        socket.shutdown().await.unwrap();
        let mut rest = Vec::new();
        socket.read_to_end(&mut rest).await.ok();
    });
    (base, server)
}

fn padded_reply(total: usize) -> Vec<u8> {
    let json = br#"{"result": {"transformed": false}}"#;
    let mut body = json.to_vec();
    body.resize(total, b' ');
    let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n").into_bytes();
    reply.extend_from_slice(&body);
    reply
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_reads_a_body_of_exactly_the_limit() {
    let (base, _server) = serve_raw(padded_reply(LIMIT)).await;
    let guardrail = handler_for(&base, None);
    let result = call_guard(&guardrail, &payload(), "test")
        .await
        .expect("a body of exactly one MiB is read whole");
    assert_eq!(result.transformed, Some(false));
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_rejects_a_declared_length_over_the_limit() {
    let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n{}".to_vec();
    let (base, _server) = serve_raw(reply).await;
    let guardrail = handler_for(&base, None);
    let error = call_guard(&guardrail, &payload(), "test")
        .await
        .expect_err("the declared length is over the limit");
    assert!(matches!(
        error,
        CrowdStrikeError::ResponseTooLarge { limit: LIMIT }
    ));
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_rejects_a_streamed_body_over_the_limit() {
    let mut reply = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..3 {
        reply.extend_from_slice(b"61A80\r\n");
        reply.resize(reply.len() + 400_000, b' ');
        reply.extend_from_slice(b"\r\n");
    }
    reply.extend_from_slice(b"0\r\n\r\n");
    let (base, _server) = serve_raw(reply).await;
    let guardrail = handler_for(&base, None);
    let error = call_guard(&guardrail, &payload(), "test")
        .await
        .expect_err("the streamed body is over the limit");
    assert!(matches!(
        error,
        CrowdStrikeError::ResponseTooLarge { limit: LIMIT }
    ));
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_reports_a_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.read(&mut [0_u8; 8192]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });
    let mut guardrail = handler_for(&base, None);
    guardrail.settings.timeout = Some(0.2);
    let error = call_guard(&guardrail, &payload(), "test")
        .await
        .expect_err("a reply that never comes times out");
    assert!(matches!(error, CrowdStrikeError::Timeout { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn _call_crowdstrike_aidr_guard_reports_a_refused_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let guardrail = handler_for(&base, None);
    let error = call_guard(&guardrail, &payload(), "test")
        .await
        .expect_err("a refused connection fails the call");
    assert!(matches!(error, CrowdStrikeError::Request { .. }));
}
