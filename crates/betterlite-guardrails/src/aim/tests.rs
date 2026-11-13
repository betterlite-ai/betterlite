#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use futures_util::StreamExt as _;
use futures_util::stream;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use secrecy::ExposeSecret as _;
use serde_json::json;
use tokio::sync::mpsc;

use super::error::{AimError, RejectionCode};
use super::output::AimOutput;
use super::settings::{AimGuardrail, AimHook, AimParams};
use super::stream::{
    AimFrameReader, AimFrameWriter, AimSocket, AimSocketConnector, AimSocketTimeouts,
    AimStreamCall, AimStreamChunk,
};
use super::transport::{AimHttpResponse, AimPoster};

const KEY: &str = "aim-key";
const BASE: &str = "http://aim.test";

struct RecordedPost {
    url: String,
    headers: HeaderMap,
    body: Vec<u8>,
    timeout: Option<Duration>,
}

struct RecordingPoster {
    status: StatusCode,
    body: Vec<u8>,
    posts: Mutex<Vec<RecordedPost>>,
}

impl Default for RecordingPoster {
    fn default() -> Self {
        Self {
            status: StatusCode::OK,
            body: Vec::new(),
            posts: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AimPoster for RecordingPoster {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        timeout: Option<Duration>,
    ) -> Result<AimHttpResponse, AimError> {
        let mut posts = self.posts.lock().unwrap_or_else(PoisonError::into_inner);
        posts.push(RecordedPost {
            url: url.to_owned(),
            headers: headers.clone(),
            body: body.to_vec(),
            timeout,
        });
        Ok(AimHttpResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}

#[derive(Default)]
struct ScriptedFrames {
    incoming: Mutex<VecDeque<Option<String>>>,
    sent: Mutex<Vec<String>>,
}

struct FakeReader {
    frames: Arc<ScriptedFrames>,
}

#[async_trait]
impl AimFrameReader for FakeReader {
    async fn recv_text(&mut self) -> Result<Option<String>, AimError> {
        let mut incoming = self
            .frames
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(incoming.pop_front().flatten())
    }
}

struct FakeWriter {
    frames: Arc<ScriptedFrames>,
}

#[async_trait]
impl AimFrameWriter for FakeWriter {
    async fn send_text(&mut self, text: String) -> Result<(), AimError> {
        let mut sent = self
            .frames
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        sent.push(text);
        Ok(())
    }
}

#[derive(Default)]
struct FakeConnector {
    frames: Arc<ScriptedFrames>,
    connections: Mutex<Vec<(String, HeaderMap)>>,
}

#[async_trait]
impl AimSocketConnector for FakeConnector {
    async fn connect(
        &self,
        url: &str,
        headers: &HeaderMap,
        _timeouts: AimSocketTimeouts,
    ) -> Result<AimSocket, AimError> {
        let mut connections = self
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        connections.push((url.to_owned(), headers.clone()));
        Ok(AimSocket {
            reader: Box::new(FakeReader {
                frames: Arc::clone(&self.frames),
            }),
            writer: Box::new(FakeWriter {
                frames: Arc::clone(&self.frames),
            }),
        })
    }
}

struct Fixture {
    guardrail: AimGuardrail,
    poster: Arc<RecordingPoster>,
    connector: Arc<FakeConnector>,
    frames: Arc<ScriptedFrames>,
}

fn fixture(response: &str, frames: &[Option<&str>]) -> Fixture {
    let poster = Arc::new(RecordingPoster {
        status: StatusCode::OK,
        body: response.as_bytes().to_vec(),
        posts: Mutex::new(Vec::new()),
    });
    let scripted = Arc::new(ScriptedFrames::default());
    {
        let mut incoming = scripted
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for frame in frames {
            incoming.push_back(frame.map(str::to_owned));
        }
    }
    let connector = Arc::new(FakeConnector {
        frames: Arc::clone(&scripted),
        connections: Mutex::new(Vec::new()),
    });
    let environment = BTreeMap::from([("AIM_API_KEY".to_owned(), KEY.to_owned())]);
    let params = AimParams {
        guardrail_name: Some("aim-safety".to_owned()),
        api_key: None,
        api_base: Some(BASE.to_owned()),
        inspect_embeddings: None,
        litellm_version: None,
    };
    let poster_port: Arc<dyn AimPoster> = Arc::clone(&poster);
    let connector_port: Arc<dyn AimSocketConnector> = Arc::clone(&connector);
    let guardrail = AimGuardrail::new(
        params,
        &environment,
        poster_port,
        connector_port,
        Some(Duration::from_secs(30)),
    )
    .unwrap();
    Fixture {
        guardrail,
        poster,
        connector,
        frames: scripted,
    }
}

fn header_value<'headers>(headers: &'headers HeaderMap, name: &str) -> Option<&'headers str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn aim_guardrail_posts_role_projected_inspection_messages() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "monitor_action"}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> = serde_json::from_value(json!({
        "messages": [
            {"role": "system", "content": "be nice"},
            {"role": "tool", "content": "tool output"},
            {"role": "function", "content": "function output"},
            {"role": "assistant", "content": "sure"},
            {"role": "user", "content": [{"type": "text", "text": "hello"}, {"type": "image_url", "image_url": {"url": "http://image"}}]},
            {"role": "user", "content": ""}
        ],
        "litellm_call_id": "call-1",
        "metadata": {"headers": {"x-aim-user-email": "user@example.com"}}
    }))
    .unwrap();
    fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, Some("alias"))
        .await
        .unwrap();
    let posts = fixture
        .poster
        .posts
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, "http://aim.test/fw/v1/analyze");
    assert_eq!(posts[0].timeout, Some(Duration::from_secs(30)));
    let body: serde_json::Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "messages": [
                {"role": "system", "content": "be nice"},
                {"role": "user", "content": "tool output"},
                {"role": "user", "content": "function output"},
                {"role": "assistant", "content": "sure"},
                {"role": "user", "content": "hello"}
            ]
        })
    );
    let headers = &posts[0].headers;
    assert_eq!(
        header_value(headers, "authorization"),
        Some("Bearer aim-key")
    );
    assert_eq!(
        header_value(headers, "x-aim-litellm-hook"),
        Some("pre_call")
    );
    assert_eq!(
        header_value(headers, "x-aim-litellm-version"),
        Some("1.105.1")
    );
    assert_eq!(header_value(headers, "x-aim-call-id"), Some("call-1"));
    assert_eq!(
        header_value(headers, "x-aim-user-email"),
        Some("user@example.com")
    );
    assert_eq!(
        header_value(headers, "x-aim-gateway-key-alias"),
        Some("alias")
    );
}

#[tokio::test]
async fn aim_guardrail_omits_absent_optional_headers() {
    let fixture = fixture(r#"{"required_action": {}}"#, &[]);
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::Moderation, None)
        .await
        .unwrap();
    let posts = fixture
        .poster
        .posts
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let headers = &posts[0].headers;
    assert_eq!(
        header_value(headers, "x-aim-litellm-hook"),
        Some("moderation")
    );
    assert_eq!(header_value(headers, "x-aim-call-id"), None);
    assert_eq!(header_value(headers, "x-aim-user-email"), None);
    assert_eq!(header_value(headers, "x-aim-gateway-key-alias"), None);
}

#[tokio::test]
async fn aim_guardrail_monitor_action_leaves_the_request_unchanged() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "monitor_action", "detection_message": "seen"}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    let before = data.clone();
    fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap();
    assert_eq!(data, before);
}

#[tokio::test]
async fn aim_guardrail_no_required_action_leaves_the_request_unchanged() {
    let fixture = fixture("{}", &[]);
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    let before = data.clone();
    fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap();
    assert_eq!(data, before);
}

#[tokio::test]
async fn aim_guardrail_block_action_is_a_content_policy_rejection() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "block_action", "detection_message": "blocked by policy"}, "analysis_result": {"policy_drill_down": {}}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "bad"}]})).unwrap();
    let error = fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap_err();
    let AimError::Rejected { message, code } = error else {
        panic!("expected a rejection, got another error");
    };
    assert_eq!(message, "blocked by policy");
    assert_eq!(code, RejectionCode::ContentPolicyViolation);
    assert_eq!(code.as_str(), "content_policy_violation");
}

#[tokio::test]
async fn aim_guardrail_block_action_survives_a_missing_analysis_result() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "block_action", "detection_message": "blocked by policy"}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "bad"}]})).unwrap();
    let error = fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AimError::Rejected { .. }));
}

#[tokio::test]
async fn aim_guardrail_anonymize_rewrites_messages_and_input() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "redacted secret"}]}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> = serde_json::from_value(json!({
        "messages": [{"role": "user", "content": "raw secret"}],
        "input": "raw secret"
    }))
    .unwrap();
    fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap();
    assert_eq!(
        data.get("messages"),
        Some(&GuardrailValue::Array(vec![GuardrailValue::Object(
            BTreeMap::from([
                ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
                (
                    "content".to_owned(),
                    GuardrailValue::Text("redacted secret".to_owned())
                ),
            ])
        )]))
    );
    assert_eq!(
        data.get("input"),
        Some(&GuardrailValue::Text("redacted secret".to_owned()))
    );
}

#[tokio::test]
async fn aim_guardrail_anonymize_rejects_multimodal_input() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "redacted"}]}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> = serde_json::from_value(json!({
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}, {"type": "image_url", "image_url": {"url": "http://image"}}]}]
    }))
    .unwrap();
    let before = data.clone();
    let error = fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap_err();
    let AimError::Rejected { code, .. } = error else {
        panic!("expected a rejection, got another error");
    };
    assert_eq!(code, RejectionCode::BadRequest);
    assert_eq!(code.as_str(), "400");
    assert_eq!(data, before);
}

#[tokio::test]
async fn aim_guardrail_anonymize_rejects_a_different_batch_size() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "redacted"}]}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> = serde_json::from_value(json!({
        "messages": [{"role": "user", "content": "one"}, {"role": "user", "content": "two"}]
    }))
    .unwrap();
    let before = data.clone();
    let error = fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AimError::Rejected {
            code: RejectionCode::BadRequest,
            ..
        }
    ));
    assert_eq!(data, before);
}

#[tokio::test]
async fn aim_guardrail_anonymize_rewrites_a_string_batch_by_element() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "first"}, {"role": "user", "content": "second"}]}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"input": ["one", "", "two"]})).unwrap();
    fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap();
    assert_eq!(
        data.get("input"),
        Some(&GuardrailValue::Array(vec![
            GuardrailValue::Text("first".to_owned()),
            GuardrailValue::Text(String::new()),
            GuardrailValue::Text("second".to_owned()),
        ]))
    );
}

#[tokio::test]
async fn aim_guardrail_anonymize_skips_a_response_without_redacted_chat() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    let before = data.clone();
    fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap();
    assert_eq!(data, before);
}

#[tokio::test]
async fn aim_guardrail_anonymize_rejects_malformed_redacted_messages() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user"}]}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    let error = fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AimError::Rejected {
            code: RejectionCode::BadRequest,
            ..
        }
    ));
}

#[tokio::test]
async fn aim_guardrail_output_anonymize_returns_the_redacted_message() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "redacted question"}, {"role": "assistant", "content": "redacted answer"}]}}"#,
        &[],
    );
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "question"}]}))
            .unwrap();
    let output = fixture
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap();
    assert_eq!(
        output,
        AimOutput::Redacted {
            output: "redacted answer".to_owned()
        }
    );
    let posts = fixture
        .poster
        .posts
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let body: serde_json::Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(
        body["messages"][1],
        json!({"role": "assistant", "content": "answer"})
    );
    assert_eq!(
        header_value(&posts[0].headers, "x-aim-litellm-hook"),
        Some("output")
    );
}

#[tokio::test]
async fn aim_guardrail_output_block_returns_the_detection_message() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "block_action", "detection_message": "blocked output"}}"#,
        &[],
    );
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "question"}]}))
            .unwrap();
    let output = fixture
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap();
    assert_eq!(
        output,
        AimOutput::Blocked {
            detection_message: Some("blocked output".to_owned())
        }
    );
}

#[tokio::test]
async fn aim_guardrail_output_without_an_action_is_unchanged() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "monitor_action"}}"#,
        &[],
    );
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "question"}]}))
            .unwrap();
    let output = fixture
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap();
    assert_eq!(output, AimOutput::Unchanged);
}

#[tokio::test]
async fn aim_guardrail_output_anonymize_rejects_a_different_count() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "assistant", "content": "redacted"}]}}"#,
        &[],
    );
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "question"}]}))
            .unwrap();
    let error = fixture
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AimError::Rejected {
            code: RejectionCode::BadRequest,
            ..
        }
    ));
}

#[tokio::test]
async fn aim_guardrail_output_anonymize_rejects_an_empty_redaction() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "redacted question"}, {"role": "assistant", "content": ""}]}}"#,
        &[],
    );
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "question"}]}))
            .unwrap();
    let error = fixture
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AimError::Rejected {
            code: RejectionCode::BadRequest,
            ..
        }
    ));
}

#[tokio::test]
async fn aim_guardrail_stream_yields_verified_chunks_in_order() {
    let first = r#"{"verified_chunk": {"id": "chatcmpl-1", "choices": [{"index": 0, "delta": {"content": "he"}}]}}"#;
    let second = r#"{"verified_chunk": {"id": "chatcmpl-1", "choices": [{"index": 0, "delta": {"content": "llo"}}]}}"#;
    let fixture = fixture(
        "{}",
        &[Some(first), Some(second), Some(r#"{"done": true}"#)],
    );
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    let chunks = stream::iter(vec![
        AimStreamChunk(r#"{"choices":[]}"#.to_owned()),
        AimStreamChunk(r#"{"choices":[]}"#.to_owned()),
    ]);
    let call = AimStreamCall {
        request_data: &request,
        key_alias: Some("alias"),
    };
    let stream = fixture
        .guardrail
        .inspect_output_stream(chunks, call)
        .await
        .unwrap();
    let mut stream = Box::pin(stream);
    let mut contents = Vec::new();
    while let Some(item) = stream.next().await {
        let chunk = item.unwrap();
        let content = chunk
            .choices
            .first()
            .and_then(|choice| choice.delta.content.clone());
        contents.push(content.unwrap_or_default());
    }
    assert_eq!(contents, ["he", "llo"]);
    let sent = fixture
        .frames
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(sent.len(), 3);
    assert_eq!(sent.last().map(String::as_str), Some(r#"{"done":true}"#));
    let connections = fixture
        .connector
        .connections
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(connections[0].0, "ws://aim.test/fw/v1/analyze/stream");
    assert_eq!(
        header_value(&connections[0].1, "x-aim-litellm-hook"),
        Some("output")
    );
}

#[tokio::test]
async fn aim_guardrail_stream_blocking_message_is_an_error() {
    let fixture = fixture(
        "{}",
        &[
            Some(r#"{"verified_chunk": {"choices": [{"delta": {"content": "he"}}]}}"#),
            Some(r#"{"blocking_message": "blocked output"}"#),
        ],
    );
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    let chunks = stream::iter(vec![AimStreamChunk(r#"{"choices":[]}"#.to_owned())]);
    let call = AimStreamCall {
        request_data: &request,
        key_alias: None,
    };
    let stream = fixture
        .guardrail
        .inspect_output_stream(chunks, call)
        .await
        .unwrap();
    let mut stream = Box::pin(stream);
    let mut contents = Vec::new();
    let mut blocked = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => contents.push(
                chunk
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.content.clone())
                    .unwrap_or_default(),
            ),
            Err(AimError::StreamBlocked { message }) => blocked = Some(message),
            Err(other) => panic!("unexpected stream error: {other}"),
        }
    }
    assert_eq!(contents, ["he"]);
    assert_eq!(blocked.as_deref(), Some("blocked output"));
}

#[tokio::test]
async fn aim_guardrail_stream_cancels_the_sender_when_aim_stops_sending_chunks() {
    let fixture = fixture(
        "{}",
        &[
            Some(r#"{"verified_chunk": {"choices": [{"delta": {"content": "he"}}]}}"#),
            Some(r#"{"done": true}"#),
        ],
    );
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    let (sender, receiver) = mpsc::channel::<AimStreamChunk>(1);
    assert!(
        sender
            .try_send(AimStreamChunk(r#"{"choices":[]}"#.to_owned()))
            .is_ok()
    );
    let chunks = stream::poll_fn(move |context| receiver.poll_recv(context));
    let call = AimStreamCall {
        request_data: &request,
        key_alias: None,
    };
    let stream = fixture
        .guardrail
        .inspect_output_stream(chunks, call)
        .await
        .unwrap();
    let mut stream = Box::pin(stream);
    while let Some(_item) = stream.next().await {}
    let closed = tokio::time::timeout(Duration::from_secs(1), sender.closed()).await;
    assert!(closed.is_ok());
}

#[tokio::test]
async fn aim_guardrail_stream_closed_early_is_an_error() {
    let fixture = fixture("{}", &[]);
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "hi"}]})).unwrap();
    let chunks = stream::iter(Vec::<AimStreamChunk>::new());
    let call = AimStreamCall {
        request_data: &request,
        key_alias: None,
    };
    let stream = fixture
        .guardrail
        .inspect_output_stream(chunks, call)
        .await
        .unwrap();
    let mut stream = Box::pin(stream);
    let item = stream.next().await.unwrap();
    assert!(matches!(item, Err(AimError::StreamClosed)));
}

#[test]
fn aim_guardrail_missing_api_key_is_an_error() {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let poster: Arc<dyn AimPoster> = Arc::new(RecordingPoster::default());
    let connector: Arc<dyn AimSocketConnector> = Arc::new(FakeConnector::default());
    let result = AimGuardrail::new(AimParams::default(), &environment, poster, connector, None);
    assert!(matches!(result, Err(AimError::MissingSecrets)));
}

#[test]
fn aim_guardrail_config_reads_the_api_key_and_default_base() {
    let source =
        "guardrails:\n  - guardrail_name: aim-safety\n    litellm_params:\n      guardrail: aim\n";
    let loader = ConfigLoader {
        lookup: BTreeMap::from([("AIM_API_KEY".to_owned(), "config-key".to_owned())]),
    };
    let config = loader.load(source).unwrap();
    let settings = &config.sections.aim_inspection;
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.guardrail_name.as_deref(), Some("aim-safety"));
    assert_eq!(configured.api_key.expose_secret(), "config-key");
    assert_eq!(configured.api_base, "https://api.aim.security");
    assert!(!configured.inspect_embeddings);
}

#[test]
fn aim_guardrail_config_reads_the_base_and_embedding_flag() {
    let source = "guardrails:\n  - guardrail_name: aim-safety\n    litellm_params:\n      guardrail: aim\n      api_key: literal-key\n      api_base: https://aim.example\n      inspect_embeddings: true\n";
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = loader.load(source).unwrap();
    let settings = &config.sections.aim_inspection;
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_base, "https://aim.example");
    assert!(configured.inspect_embeddings);
    assert_eq!(settings.guardrails.len(), 1);
}

#[test]
fn aim_guardrail_config_requires_an_api_key() {
    let source =
        "guardrails:\n  - guardrail_name: aim-safety\n    litellm_params:\n      guardrail: aim\n";
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    assert!(loader.load(source).is_err());
}

#[test]
fn aim_guardrail_ws_base_follows_the_http_scheme() {
    let environment = BTreeMap::from([("AIM_API_KEY".to_owned(), KEY.to_owned())]);
    let poster: Arc<dyn AimPoster> = Arc::new(RecordingPoster::default());
    let connector: Arc<dyn AimSocketConnector> = Arc::new(FakeConnector::default());
    let secure_params = AimParams {
        guardrail_name: None,
        api_key: None,
        api_base: Some("https://api.aim.security".to_owned()),
        inspect_embeddings: None,
        litellm_version: None,
    };
    let plain_params = AimParams {
        guardrail_name: None,
        api_key: None,
        api_base: Some("http://127.0.0.1:9000".to_owned()),
        inspect_embeddings: None,
        litellm_version: None,
    };
    let secure = AimGuardrail::new(
        secure_params,
        &environment,
        Arc::clone(&poster),
        Arc::clone(&connector),
        None,
    )
    .unwrap();
    assert_eq!(secure.ws_api_base, "wss://api.aim.security");
    let plain = AimGuardrail::new(plain_params, &environment, poster, connector, None).unwrap();
    assert_eq!(plain.ws_api_base, "ws://127.0.0.1:9000");
    assert_eq!(plain.api_key.expose_secret(), KEY);
}
