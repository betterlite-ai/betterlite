#![cfg(test)]

use std::net::SocketAddr;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::passthrough_call::PassthroughBody;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use pyo3_async_runtimes::tokio::get_runtime;

use super::{SessionCore, SessionError, logging_state};
use crate::passthrough_calls::convert::{BAD_REQUEST, BAD_REQUEST_STATUS, ErrorPayload, call_args};

const ENVIRONMENT: &str = r#"{"environment": {}}"#;

fn vllm_request(address: &SocketAddr, streaming: bool) -> String {
    // LiteLLM's vLLM passthrough reads any body with a stream key as a streaming call, even stream: false: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vllm/passthrough/transformation.py#L12-L13
    let stream_key = if streaming { r#", "stream": true"# } else { "" };
    format!(
        r#"{{"method": "POST", "endpoint": "chat/completions", "model": "vllm/my-model", "custom_llm_provider": "vllm", "api_base": "http://{address}/v1", "json": {{"model": "original"{stream_key}}}}}"#
    )
}

fn call_once(core: &SessionCore, request: &str) -> Result<PassthroughReplyWire, Box<ErrorPayload>> {
    let args = call_args(request, &mut Vec::new()).map_err(|error| {
        Box::new(ErrorPayload::failed(
            BAD_REQUEST,
            Some(BAD_REQUEST_STATUS),
            error.to_string(),
        ))
    })?;
    let reply = get_runtime().block_on(core.call(args))?;
    Ok(PassthroughReplyWire {
        status: reply.status,
        headers: reply.headers,
        body: reply.body,
    })
}

// the reply stays borrowed from the runtime, so the test keeps its three observable parts
struct PassthroughReplyWire {
    status: u16,
    headers: Vec<(String, String)>,
    body: PassthroughBody,
}

#[test]
fn call_returns_the_upstream_status_headers_and_body() {
    let serving = serve(
        reply("200 OK", &[("x-upstream", "yes")], "pong"),
        Duration::ZERO,
    )
    .unwrap();
    let core = SessionCore::build(ENVIRONMENT).unwrap();
    let reply = call_once(&core, &vllm_request(&serving.address, false)).unwrap();
    assert_eq!(reply.status, 200);
    let upstream = reply
        .headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case("x-upstream"));
    assert_eq!(upstream.map(|(_name, value)| value.as_str()), Some("yes"));
    let PassthroughBody::Full(body) = reply.body else {
        panic!("a non-streaming request answers with the whole body");
    };
    assert_eq!(&body[..], b"pong");
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/chat/completions HTTP/1.1");
    assert!(
        sent.head.contains("content-type: application/json"),
        "{}",
        sent.head
    );
}

#[test]
fn call_reports_an_upstream_error_with_its_status_and_body() {
    let answer = reply("404 Not Found", &[], r#"{"error": "missing"}"#);
    let serving = serve(answer, Duration::ZERO).unwrap();
    let core = SessionCore::build(ENVIRONMENT).unwrap();
    let error = call_once(&core, &vllm_request(&serving.address, false))
        .err()
        .unwrap();
    assert_eq!(error.kind, "APIError");
    assert_eq!(error.status_code, Some(404));
    assert_eq!(error.message, r#"{"error": "missing"}"#);
    let expected_body: JsonValue = serde_json::from_str(r#"{"error": "missing"}"#).unwrap();
    assert_eq!(error.body, Some(expected_body));
}

#[test]
fn call_refuses_a_model_no_provider_serves() {
    let request = r#"{"method": "GET", "endpoint": "models", "model": "unknown-model"}"#;
    let core = SessionCore::build(ENVIRONMENT).unwrap();
    let error = call_once(&core, request).err().unwrap();
    assert_eq!(error.kind, "BadRequestError");
    assert_eq!(error.status_code, Some(400));
}

#[test]
fn call_uses_the_session_environment() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}/api/v1", serving.address);
    let session = format!(
        r#"{{"environment": {{"GIGACHAT_API_BASE": "{base}", "GIGACHAT_ACCESS_TOKEN": "session-token"}}}}"#
    );
    let core = SessionCore::build(&session).unwrap();
    let request =
        r#"{"method": "POST", "endpoint": "chat/completions", "model": "gigachat/GigaChat"}"#;
    let reply = call_once(&core, request).unwrap();
    assert_eq!(reply.status, 200);
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /api/v1/chat/completions HTTP/1.1");
    assert!(sent.head.contains("authorization: Bearer session-token"));
}

#[test]
fn build_reads_the_session_environment() {
    SessionCore::build("{}").unwrap();
    SessionCore::build(r#"{"environment": {"HTTPS_PROXY": "http://proxy.example:3128"}}"#).unwrap();
}

#[test]
fn build_refuses_a_text_that_is_not_a_session() {
    let error = SessionCore::build("not a session").err().unwrap();
    assert!(matches!(error, SessionError::Json(_)));
}

#[test]
fn logging_state_names_the_model_and_the_call_type() {
    let state = logging_state("vllm/my-model").unwrap();
    assert_eq!(state.model, "vllm/my-model");
    assert_eq!(state.call_type, "passthrough");
    assert_eq!(state.messages, None);
    assert_eq!(state.stream, None);
    assert!(!state.litellm_call_id.is_empty());
    assert!(!state.litellm_trace_id.is_empty());
}
