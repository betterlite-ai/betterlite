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

use super::error::{AimError, CONTENT_POLICY_VIOLATION};
use super::output::AimOutput;
use super::settings::{AimGuardrail, AimHook, AimParams};
use super::stream::{
    AimFrameReader, AimFrameWriter, AimSocket, AimSocketConnector, AimStreamChunk,
};
use super::transport::{AimHttpResponse, AimPoster};

const KEY: &str = "aim-key";

#[derive(Default)]
struct State {
    posts: Mutex<Vec<(String, HeaderMap, Vec<u8>)>>,
    incoming: Mutex<VecDeque<Option<String>>>,
    sent: Mutex<Vec<String>>,
}

struct Fake {
    state: Arc<State>,
    status: StatusCode,
    body: Vec<u8>,
}

#[async_trait]
impl AimPoster for Fake {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        _timeout: Option<Duration>,
    ) -> Result<AimHttpResponse, AimError> {
        let mut posts = self
            .state
            .posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        posts.push((url.to_owned(), headers.clone(), body.to_vec()));
        Ok(AimHttpResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}

#[async_trait]
impl AimFrameReader for Fake {
    async fn recv_text(&mut self) -> Result<Option<String>, AimError> {
        let mut incoming = self
            .state
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(incoming.pop_front().flatten())
    }
}

#[async_trait]
impl AimFrameWriter for Fake {
    async fn send_text(&mut self, text: String) -> Result<(), AimError> {
        let mut sent = self
            .state
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        sent.push(text);
        Ok(())
    }
}

#[async_trait]
impl AimSocketConnector for Fake {
    async fn connect(
        &self,
        url: &str,
        headers: &HeaderMap,
        _open: Duration,
        _read: Duration,
    ) -> Result<AimSocket, AimError> {
        let mut posts = self
            .state
            .posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        posts.push((url.to_owned(), headers.clone(), Vec::new()));
        Ok(AimSocket {
            reader: Box::new(Fake {
                state: Arc::clone(&self.state),
                status: self.status,
                body: Vec::new(),
            }),
            writer: Box::new(Fake {
                state: Arc::clone(&self.state),
                status: self.status,
                body: Vec::new(),
            }),
        })
    }
}

struct Fixture {
    guardrail: AimGuardrail,
    state: Arc<State>,
}

fn fixture(response: &str, frames: &[Option<&str>]) -> Fixture {
    let state = Arc::new(State::default());
    {
        let mut incoming = state
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for frame in frames {
            incoming.push_back(frame.map(str::to_owned));
        }
    }
    let fake = Arc::new(Fake {
        state: Arc::clone(&state),
        status: StatusCode::OK,
        body: response.as_bytes().to_vec(),
    });
    let environment = BTreeMap::from([("AIM_API_KEY".to_owned(), KEY.to_owned())]);
    let params = AimParams {
        guardrail_name: Some("aim-safety".to_owned()),
        api_key: None,
        api_base: Some("http://aim.test".to_owned()),
        inspect_embeddings: None,
        litellm_version: None,
    };
    let poster: Arc<dyn AimPoster> = Arc::clone(&fake);
    let connector: Arc<dyn AimSocketConnector> = fake;
    let guardrail = AimGuardrail::new(
        params,
        &environment,
        poster,
        connector,
        Some(Duration::from_secs(30)),
    )
    .unwrap();
    Fixture { guardrail, state }
}

fn body_of(state: &State) -> serde_json::Value {
    let posts = state.posts.lock().unwrap_or_else(PoisonError::into_inner);
    serde_json::from_slice(&posts[0].2).unwrap()
}

fn header_of(state: &State, name: &str) -> Option<String> {
    let posts = state.posts.lock().unwrap_or_else(PoisonError::into_inner);
    let headers = &posts[0].1;
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn rejection(error: AimError) -> &'static str {
    let AimError::Rejected { code, .. } = error else {
        panic!("expected a rejection");
    };
    code
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
            {"role": "user", "content": [{"type": "text", "text": "hello"}, {"type": "image_url", "image_url": {"url": "http://image"}}]}
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
    assert_eq!(
        body_of(&fixture.state),
        json!({"messages": [
            {"role": "system", "content": "be nice"},
            {"role": "user", "content": "tool output"},
            {"role": "user", "content": "hello"}
        ]})
    );
    assert_eq!(
        header_of(&fixture.state, "authorization").as_deref(),
        Some("Bearer aim-key")
    );
    assert_eq!(
        header_of(&fixture.state, "x-aim-litellm-hook").as_deref(),
        Some("pre_call")
    );
    assert_eq!(
        header_of(&fixture.state, "x-aim-litellm-version").as_deref(),
        Some("1.105.1")
    );
    assert_eq!(
        header_of(&fixture.state, "x-aim-call-id").as_deref(),
        Some("call-1")
    );
    assert_eq!(
        header_of(&fixture.state, "x-aim-user-email").as_deref(),
        Some("user@example.com")
    );
    assert_eq!(
        header_of(&fixture.state, "x-aim-gateway-key-alias").as_deref(),
        Some("alias")
    );
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
    assert_eq!(rejection(error), CONTENT_POLICY_VIOLATION);
    assert_eq!(CONTENT_POLICY_VIOLATION, "content_policy_violation");
}

#[tokio::test]
async fn aim_guardrail_block_action_survives_a_missing_analysis_result() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "block_action", "detection_message": "blocked"}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "bad"}]})).unwrap();
    let error = fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap_err();
    assert_eq!(rejection(error), CONTENT_POLICY_VIOLATION);
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
    assert_eq!(rejection(error), "400");
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
async fn aim_guardrail_anonymize_rejects_a_different_batch_size() {
    let fixture = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "redacted"}]}}"#,
        &[],
    );
    let mut data: BTreeMap<String, GuardrailValue> = serde_json::from_value(json!({
        "messages": [{"role": "user", "content": "one"}, {"role": "user", "content": "two"}]
    }))
    .unwrap();
    let error = fixture
        .guardrail
        .call_aim_guardrail(&mut data, AimHook::PreCall, None)
        .await
        .unwrap_err();
    assert_eq!(rejection(error), "400");
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
    assert_eq!(rejection(error), "400");
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
    assert_eq!(
        body_of(&fixture.state)["messages"][1],
        json!({"role": "assistant", "content": "answer"})
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
async fn aim_guardrail_output_anonymize_rejects_bad_redactions() {
    let request: BTreeMap<String, GuardrailValue> =
        serde_json::from_value(json!({"messages": [{"role": "user", "content": "question"}]}))
            .unwrap();
    let short = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "assistant", "content": "redacted"}]}}"#,
        &[],
    );
    let error = short
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap_err();
    assert_eq!(rejection(error), "400");
    let empty = fixture(
        r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "question"}, {"role": "assistant", "content": ""}]}}"#,
        &[],
    );
    let error = empty
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap_err();
    assert_eq!(rejection(error), "400");
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
    let call = Some("alias");
    let mut stream = Box::pin(
        fixture
            .guardrail
            .inspect_output_stream(chunks, &request, call)
            .await
            .unwrap(),
    );
    let mut contents = Vec::new();
    while let Some(item) = stream.next().await {
        let content = item
            .unwrap()
            .choices
            .first()
            .and_then(|choice| choice.delta.content.clone());
        contents.push(content.unwrap_or_default());
    }
    assert_eq!(contents, ["he", "llo"]);
    let sent = fixture
        .state
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(sent.len(), 3);
    assert_eq!(sent.last().map(String::as_str), Some(r#"{"done":true}"#));
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
    let call = None;
    let mut stream = Box::pin(
        fixture
            .guardrail
            .inspect_output_stream(chunks, &request, call)
            .await
            .unwrap(),
    );
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
    let call = None;
    let mut stream = Box::pin(
        fixture
            .guardrail
            .inspect_output_stream(chunks, &request, call)
            .await
            .unwrap(),
    );
    while let Some(_item) = stream.next().await {}
    let closed = tokio::time::timeout(Duration::from_secs(1), sender.closed()).await;
    assert!(closed.is_ok());
}

#[test]
fn aim_guardrail_missing_api_key_is_an_error() {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let state = Arc::new(State::default());
    let poster: Arc<dyn AimPoster> = Arc::new(Fake {
        state: Arc::clone(&state),
        status: StatusCode::OK,
        body: Vec::new(),
    });
    let connector: Arc<dyn AimSocketConnector> = Arc::new(Fake {
        state,
        status: StatusCode::OK,
        body: Vec::new(),
    });
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
    let configured = config.sections.aim_inspection.guardrails.first().unwrap();
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
    let configured = config.sections.aim_inspection.guardrails.first().unwrap();
    assert_eq!(configured.api_base, "https://aim.example");
    assert!(configured.inspect_embeddings);
}
