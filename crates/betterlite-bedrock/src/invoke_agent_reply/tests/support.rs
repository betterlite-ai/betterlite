use std::io;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::agentcore_response::transform::RawResponse;
use crate::invoke_agent_reply::decode_agent_trace;
use crate::invoke_agent_reply::error::InvokeAgentReplyError;
use crate::invoke_agent_reply::events::AgentEvent;
use crate::invoke_agent_reply::reply::transform_invoke_agent_response;
use crate::invoke_agent_reply::wire::ModelResponse;

const TEXT_HEADER: u8 = 7;
const INTEGER_HEADER: u8 = 4;

pub(super) enum Header<'name> {
    Text(&'name str, &'name str),
    Integer(&'name str, i32),
}

pub(super) fn frame(headers: &[Header<'_>], payload: &[u8]) -> Vec<u8> {
    let encoded_headers = encoded_headers(headers);
    let total = 12_usize
        .checked_add(encoded_headers.len())
        .and_then(|value| value.checked_add(payload.len()))
        .and_then(|value| value.checked_add(4))
        .unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u32::try_from(total).unwrap().to_be_bytes());
    bytes.extend_from_slice(&u32::try_from(encoded_headers.len()).unwrap().to_be_bytes());
    let prelude_crc = crc32fast::hash(&bytes);
    bytes.extend_from_slice(&prelude_crc.to_be_bytes());
    bytes.extend_from_slice(&encoded_headers);
    bytes.extend_from_slice(payload);
    let message_crc = crc32fast::hash(&bytes);
    bytes.extend_from_slice(&message_crc.to_be_bytes());
    bytes
}

pub(super) fn chunk(text: &str) -> Vec<u8> {
    chunk_body(&format!("{{\"bytes\":\"{}\"}}", STANDARD.encode(text)))
}

pub(super) fn chunk_body(body: &str) -> Vec<u8> {
    event_frame("chunk", "event", body.as_bytes())
}

pub(super) fn trace(body: &str) -> Vec<u8> {
    event_frame("trace", "event", body.as_bytes())
}

pub(super) fn event_frame(event_type: &str, message_type: &str, payload: &[u8]) -> Vec<u8> {
    frame(
        &[
            Header::Text(":event-type", event_type),
            Header::Text(":content-type", "application/json"),
            Header::Text(":message-type", message_type),
        ],
        payload,
    )
}

pub(super) fn join(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn supplied() -> ModelResponse {
    ModelResponse {
        id: "chatcmpl-kept".to_owned(),
        created: 7,
        model: Some("previous-model".to_owned()),
        object: "chat.completion".to_owned(),
        system_fingerprint: Some("fp-1".to_owned()),
        choices: Vec::new(),
        usage: None,
        service_tier: Some(json("\"priority\"")),
        trace: Some(json("{\"guardrail\":\"kept\"}")),
    }
}

pub(super) fn raw(status: u16, body: &[u8]) -> RawResponse {
    RawResponse {
        status_code: status,
        headers: vec![(
            "content-type".to_owned(),
            "application/vnd.amazon.eventstream".to_owned(),
        )],
        body: body.to_vec(),
    }
}

pub(super) fn run(status: u16, body: &[u8]) -> (Result<(), InvokeAgentReplyError>, ModelResponse) {
    let mut response = supplied();
    let outcome = transform_invoke_agent_response(&raw(status, body), &mut response);
    (outcome, response)
}

pub(super) fn completed(status: u16, body: &[u8]) -> ModelResponse {
    let (outcome, response) = run(status, body);
    outcome.unwrap();
    response
}

pub(super) fn events(body: &[u8]) -> Vec<AgentEvent> {
    decode_agent_trace(body).unwrap()
}

pub(super) fn content(response: &ModelResponse) -> &str {
    response
        .choices
        .first()
        .unwrap()
        .message
        .content
        .as_deref()
        .unwrap()
}

pub(super) fn usage(response: &ModelResponse) -> (i64, i64, i64) {
    let usage = response.usage.as_ref().unwrap();
    (
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.total_tokens,
    )
}

pub(super) fn bedrock_status(outcome: Result<(), InvokeAgentReplyError>) -> u16 {
    match outcome {
        Err(InvokeAgentReplyError::Bedrock { status, error }) => {
            assert!(
                error
                    .cause
                    .error
                    .message
                    .starts_with("Error processing response: ")
            );
            assert_eq!(error.cause.error.status_code, status);
            assert!(!error.cause.error.status_code_is_synthesized);
            status
        }
        Ok(()) | Err(_) => panic!("expected a bedrock failure"),
    }
}

fn encoded_headers(headers: &[Header<'_>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for header in headers {
        match header {
            Header::Text(name, value) => text_header(&mut bytes, name, value),
            Header::Integer(name, value) => integer_header(&mut bytes, name, *value),
        }
    }
    bytes
}

fn text_header(bytes: &mut Vec<u8>, name: &str, value: &str) {
    bytes.push(u8::try_from(name.len()).unwrap());
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(TEXT_HEADER);
    bytes.extend_from_slice(&u16::try_from(value.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn integer_header(bytes: &mut Vec<u8>, name: &str, value: i32) {
    bytes.push(u8::try_from(name.len()).unwrap());
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(INTEGER_HEADER);
    bytes.extend_from_slice(&value.to_be_bytes());
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
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

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
