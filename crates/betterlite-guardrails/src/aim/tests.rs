#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use futures_util::StreamExt as _;
use futures_util::stream::{self, Stream};
use reqwest::header::HeaderMap;
use reqwest::{ClientBuilder, StatusCode};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::{JoinSet, yield_now};

use super::error::{AimError, CONTENT_POLICY_VIOLATION};
use super::input;
use super::output::AimOutput;
use super::settings::{AimGuardrail, AimHook, AimParams};
use super::stream::{
    AimFrameReader, AimFrameWriter, AimSocket, AimSocketConnector, AimStreamChunk,
    MAX_STREAM_FRAME_BYTES, VerifiedChunk,
};
use super::transport::{AimHttpResponse, AimPoster, HttpPoster, MAX_RESPONSE_BYTES};
use super::wire::{AimAction, AnalyzeResponse, action_of};

const KEY: &str = "aim-key";

#[derive(Default)]
struct State {
    posts: Mutex<Vec<(String, HeaderMap, Vec<u8>)>>,
    incoming: Mutex<VecDeque<Option<String>>>,
    sent: Mutex<Vec<String>>,
}

impl State {
    fn record(&self, url: &str, headers: &HeaderMap, body: &[u8]) {
        let mut posts = self.posts.lock().unwrap_or_else(PoisonError::into_inner);
        posts.push((url.to_owned(), headers.clone(), body.to_vec()));
    }
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
        self.state.record(url, headers, body);
        Ok(AimHttpResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}

#[async_trait]
impl AimFrameReader for Fake {
    async fn recv_text(&mut self) -> Result<Option<String>, AimError> {
        yield_now().await;
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
        self.state.record(url, headers, &[]);
        let half = || Fake {
            state: Arc::clone(&self.state),
            status: self.status,
            body: Vec::new(),
        };
        Ok(AimSocket {
            reader: Box::new(half()),
            writer: Box::new(half()),
        })
    }
}

struct Fixture {
    guardrail: AimGuardrail,
    state: Arc<State>,
}

fn guardrail_with(
    params: AimParams,
    environment: &BTreeMap<String, String>,
    state: &Arc<State>,
    reply: (StatusCode, &str),
) -> Result<AimGuardrail, AimError> {
    let fake = Arc::new(Fake {
        state: Arc::clone(state),
        status: reply.0,
        body: reply.1.as_bytes().to_vec(),
    });
    let poster: Arc<dyn AimPoster> = Arc::clone(&fake);
    let connector: Arc<dyn AimSocketConnector> = fake;
    AimGuardrail::new(
        params,
        environment,
        poster,
        connector,
        Some(Duration::from_secs(30)),
    )
}

fn fixture(status: StatusCode, response: &str, frames: &[Option<&str>]) -> Fixture {
    let state = Arc::new(State::default());
    state
        .incoming
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .extend(frames.iter().map(|frame| frame.map(str::to_owned)));
    let environment = BTreeMap::from([("AIM_API_KEY".to_owned(), KEY.to_owned())]);
    let params = AimParams {
        guardrail_name: Some("aim-safety".to_owned()),
        api_base: Some("http://aim.test".to_owned()),
        ..AimParams::default()
    };
    let guardrail = guardrail_with(params, &environment, &state, (status, response)).unwrap();
    Fixture { guardrail, state }
}

fn data(text: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(text).unwrap()
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
        StatusCode::OK,
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
        StatusCode::OK,
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
        StatusCode::OK,
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
        StatusCode::OK,
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
        StatusCode::OK,
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
        StatusCode::OK,
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
        StatusCode::OK,
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
        StatusCode::OK,
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
        StatusCode::OK,
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
        StatusCode::OK,
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

#[rstest]
#[case::short(
    r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "assistant", "content": "redacted"}]}}"#
)]
#[case::empty(
    r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "user", "content": "question"}, {"role": "assistant", "content": ""}]}}"#
)]
#[case::without_a_chat(r#"{"required_action": {"action_type": "anonymize_action"}}"#)]
#[case::malformed(
    r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {"all_redacted_messages": [{"role": "assistant"}]}}"#
)]
#[tokio::test]
async fn aim_guardrail_output_anonymize_rejects_bad_redactions(#[case] response: &str) {
    let request = data(r#"{"messages": [{"role": "user", "content": "question"}]}"#);
    let error = fixture(StatusCode::OK, response, &[])
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap_err();
    assert_eq!(rejection(error), "400");
}

#[rstest]
#[case::no_action("{}")]
#[case::monitor(r#"{"required_action": {"action_type": "monitor_action"}}"#)]
#[case::unknown(r#"{"required_action": {"action_type": "mystery"}}"#)]
#[tokio::test]
async fn aim_guardrail_output_without_an_action_is_unchanged(#[case] response: &str) {
    let request = data(r#"{"messages": [{"role": "user", "content": "question"}]}"#);
    let output = fixture(StatusCode::OK, response, &[])
        .guardrail
        .call_aim_guardrail_on_output(&request, "answer", None)
        .await
        .unwrap();
    assert_eq!(output, AimOutput::Unchanged);
}

#[rstest]
#[case::no_action("{}")]
#[case::monitor(r#"{"required_action": {"action_type": "monitor_action"}}"#)]
#[case::action_without_a_type(r#"{"required_action": {}}"#)]
#[case::unknown(r#"{"required_action": {"action_type": "mystery"}}"#)]
#[case::anonymize_without_a_chat(r#"{"required_action": {"action_type": "anonymize_action"}}"#)]
#[case::anonymize_without_messages(
    r#"{"required_action": {"action_type": "anonymize_action"}, "redacted_chat": {}}"#
)]
#[tokio::test]
async fn aim_guardrail_leaves_the_request_unchanged_without_a_rewrite(#[case] response: &str) {
    let mut request =
        data(r#"{"messages": [{"role": "user", "content": "keep"}], "input": "keep"}"#);
    let before = request.clone();
    fixture(StatusCode::OK, response, &[])
        .guardrail
        .call_aim_guardrail(&mut request, AimHook::Moderation, None)
        .await
        .unwrap();
    assert_eq!(request, before);
}

#[tokio::test]
async fn aim_guardrail_rejects_a_failed_status_and_an_unparsable_answer() {
    let mut request = data(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
    let failed = fixture(StatusCode::INTERNAL_SERVER_ERROR, "{}", &[]);
    let status_error = failed
        .guardrail
        .call_aim_guardrail(&mut request, AimHook::PreCall, None)
        .await
        .unwrap_err();
    assert!(
        matches!(status_error, AimError::HttpStatus(status) if status == StatusCode::INTERNAL_SERVER_ERROR)
    );
    let garbled = fixture(StatusCode::OK, "not json", &[]);
    let json_error = garbled
        .guardrail
        .call_aim_guardrail(&mut request, AimHook::PreCall, None)
        .await
        .unwrap_err();
    assert!(matches!(json_error, AimError::ResponseJson(_)));
}

#[rstest]
#[case::none("{}", AimAction::None)]
#[case::untyped(r#"{"required_action": {}}"#, AimAction::None)]
#[case::monitor(
    r#"{"required_action": {"action_type": "monitor_action"}}"#,
    AimAction::Monitor
)]
#[case::block(
    r#"{"required_action": {"action_type": "block_action"}}"#,
    AimAction::Block
)]
#[case::anonymize(
    r#"{"required_action": {"action_type": "anonymize_action"}}"#,
    AimAction::Anonymize
)]
#[case::unknown(
    r#"{"required_action": {"action_type": "mystery"}}"#,
    AimAction::Unknown
)]
fn aim_guardrail_maps_the_action_type(#[case] response: &str, #[case] expected: AimAction) {
    let parsed: AnalyzeResponse = serde_json::from_str(response).unwrap();
    assert_eq!(action_of(&parsed), expected);
}

#[rstest]
#[case::pre_call(AimHook::PreCall, "pre_call")]
#[case::moderation(AimHook::Moderation, "moderation")]
#[case::output(AimHook::Output, "output")]
fn aim_guardrail_names_the_hook_in_its_header(#[case] hook: AimHook, #[case] expected: &str) {
    let guardrail = fixture(StatusCode::OK, "{}", &[]).guardrail;
    let headers = guardrail.build_headers(hook, None, None, None).unwrap();
    assert_eq!(headers.get("x-aim-litellm-hook").unwrap(), expected);
}

#[test]
fn aim_guardrail_omits_empty_and_absent_optional_headers() {
    let guardrail = fixture(StatusCode::OK, "{}", &[]).guardrail;
    let optional = [
        "x-aim-call-id",
        "x-aim-user-email",
        "x-aim-gateway-key-alias",
    ];
    for (alias, email, call) in [(None, None, None), (Some(""), Some(""), Some(""))] {
        let headers = guardrail
            .build_headers(AimHook::PreCall, alias, email, call)
            .unwrap();
        assert_eq!(headers.len(), 3);
        assert!(optional.iter().all(|name| !headers.contains_key(*name)));
    }
    let full = guardrail
        .build_headers(AimHook::PreCall, Some("a"), Some("e"), Some("c"))
        .unwrap();
    assert!(optional.iter().all(|name| full.contains_key(*name)));
    let invalid = guardrail.build_headers(AimHook::PreCall, Some("bad\nalias"), None, None);
    assert!(matches!(invalid, Err(AimError::InvalidHeader(_))));
    let empty =
        data(r#"{"litellm_call_id": "", "metadata": {"headers": {"x-aim-user-email": ""}}}"#);
    assert_eq!(input::call_id(&empty), None);
    assert_eq!(input::user_email(&empty), None);
    let absent = data(r#"{"metadata": {"headers": "none"}}"#);
    assert_eq!(input::user_email(&absent), None);
}

#[rstest]
#[case::default_base(&[("AIM_API_KEY", "k")], None, "https://api.aim.security", "wss://api.aim.security")]
#[case::env_base(&[("AIM_API_KEY", "k"), ("AIM_API_BASE", "http://env.aim")], None, "http://env.aim", "ws://env.aim")]
#[case::empty_env_base(&[("AIM_API_KEY", "k"), ("AIM_API_BASE", "")], None, "https://api.aim.security", "wss://api.aim.security")]
#[case::param_base(&[("AIM_API_KEY", "k"), ("AIM_API_BASE", "http://env.aim")], Some("https://param.aim"), "https://param.aim", "wss://param.aim")]
#[case::empty_param_base(&[("AIM_API_KEY", "k"), ("AIM_API_BASE", "http://env.aim")], Some(""), "http://env.aim", "ws://env.aim")]
fn aim_guardrail_resolves_the_base_and_the_websocket_base(
    #[case] variables: &[(&str, &str)],
    #[case] param: Option<&str>,
    #[case] base: &str,
    #[case] websocket_base: &str,
) {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let params = AimParams {
        api_base: param.map(str::to_owned),
        ..AimParams::default()
    };
    let state = Arc::new(State::default());
    let guardrail = guardrail_with(params, &environment, &state, (StatusCode::OK, "{}")).unwrap();
    assert_eq!(guardrail.api_base, base);
    assert_eq!(guardrail.ws_api_base, websocket_base);
    assert_eq!(guardrail.litellm_version, "1.105.1");
    assert!(!guardrail.inspect_embeddings);
}

#[test]
fn aim_guardrail_prefers_the_configured_api_key_over_the_environment() {
    let environment = BTreeMap::from([("AIM_API_KEY".to_owned(), "env-key".to_owned())]);
    let state = Arc::new(State::default());
    for (param, expected) in [
        (Some("param-key"), "param-key"),
        (Some(""), "env-key"),
        (None, "env-key"),
    ] {
        let params = AimParams {
            api_key: param.map(|key| SecretString::from(key.to_owned())),
            ..AimParams::default()
        };
        let guardrail =
            guardrail_with(params, &environment, &state, (StatusCode::OK, "{}")).unwrap();
        assert_eq!(guardrail.api_key.expose_secret(), expected);
    }
}

async fn drain<S>(fixture: &Fixture, chunks: S) -> Vec<Result<VerifiedChunk, AimError>>
where
    S: Stream<Item = AimStreamChunk> + Send + 'static,
{
    let request = data(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
    let stream = fixture
        .guardrail
        .inspect_output_stream(chunks, &request, Some("alias"))
        .await
        .unwrap();
    Box::pin(stream).take(8).collect().await
}

fn texts(items: &[Result<VerifiedChunk, AimError>]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|item| item.as_ref().ok())
        .map(|chunk| {
            chunk
                .choices
                .first()
                .and_then(|choice| choice.delta.content.as_deref())
                .unwrap_or_default()
        })
        .collect()
}

#[tokio::test]
async fn aim_guardrail_stream_yields_verified_chunks_in_order() {
    let first = r#"{"verified_chunk": {"id": "chatcmpl-1", "choices": [{"index": 0, "delta": {"content": "he"}}]}}"#;
    let second = r#"{"verified_chunk": {"id": "chatcmpl-1", "choices": [{"index": 0, "delta": {"content": "llo"}}]}}"#;
    let fixture = fixture(
        StatusCode::OK,
        "{}",
        &[Some(first), Some(second), Some(r#"{"done": true}"#)],
    );
    let chunks = stream::iter(vec![
        AimStreamChunk(r#"{"choices":[]}"#.to_owned()),
        AimStreamChunk(r#"{"choices":[]}"#.to_owned()),
    ]);
    let items = drain(&fixture, chunks).await;
    assert_eq!(items.len(), 2);
    assert_eq!(texts(&items), ["he", "llo"]);
    let sent = fixture
        .state
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(sent.len(), 3);
    assert_eq!(sent.last().map(String::as_str), Some(r#"{"done":true}"#));
    assert_eq!(
        header_of(&fixture.state, "x-aim-litellm-hook").as_deref(),
        Some("output")
    );
    assert_eq!(
        header_of(&fixture.state, "x-aim-gateway-key-alias").as_deref(),
        Some("alias")
    );
    let posts = fixture
        .state
        .posts
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(posts[0].0, "ws://aim.test/fw/v1/analyze/stream");
}

#[tokio::test]
async fn aim_guardrail_stream_blocking_message_is_an_error() {
    let fixture = fixture(
        StatusCode::OK,
        "{}",
        &[
            Some(r#"{"verified_chunk": {"choices": [{"delta": {"content": "he"}}]}}"#),
            Some(r#"{"blocking_message": "blocked output"}"#),
        ],
    );
    let chunks = stream::iter(vec![AimStreamChunk(r#"{"choices":[]}"#.to_owned())]);
    let items = drain(&fixture, chunks).await;
    assert_eq!(items.len(), 2);
    assert_eq!(texts(&items), ["he"]);
    assert!(
        matches!(&items[1], Err(AimError::StreamBlocked { message }) if message == "blocked output")
    );
}

#[rstest]
#[case::closed_early(&[], "the Aim stream closed before it finished")]
#[case::malformed_frame(&[Some("not json")], "an Aim stream frame is not valid JSON")]
#[tokio::test]
async fn aim_guardrail_stream_failures_end_the_stream(
    #[case] frames: &[Option<&str>],
    #[case] message: &str,
) {
    let fixture = fixture(StatusCode::OK, "{}", frames);
    let items = drain(&fixture, stream::empty()).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].as_ref().unwrap_err().to_string(), message);
}

#[tokio::test]
async fn aim_guardrail_stream_cancels_the_sender_when_aim_stops_sending_chunks() {
    let fixture = fixture(
        StatusCode::OK,
        "{}",
        &[
            Some(r#"{"verified_chunk": {"choices": [{"delta": {"content": "he"}}]}}"#),
            Some(r#"{"done": true}"#),
        ],
    );
    let (sender, receiver) = mpsc::channel::<AimStreamChunk>(1);
    assert!(
        sender
            .try_send(AimStreamChunk(r#"{"choices":[]}"#.to_owned()))
            .is_ok()
    );
    let chunks = stream::poll_fn(move |context| receiver.poll_recv(context));
    let items = drain(&fixture, chunks).await;
    assert_eq!(texts(&items), ["he"]);
    let closed = tokio::time::timeout(Duration::from_secs(1), sender.closed()).await;
    assert!(closed.is_ok());
}

#[test]
fn aim_guardrail_stream_frame_limit_is_one_mebibyte() {
    assert_eq!(MAX_STREAM_FRAME_BYTES, 1_048_576);
}

#[test]
fn aim_guardrail_missing_api_key_is_an_error() {
    let state = Arc::new(State::default());
    let result = guardrail_with(
        AimParams::default(),
        &BTreeMap::new(),
        &state,
        (StatusCode::OK, "{}"),
    );
    assert!(matches!(result, Err(AimError::MissingSecrets)));
}

struct Server {
    base: String,
    tasks: JoinSet<Vec<u8>>,
}

fn request_complete(request: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(request) else {
        return false;
    };
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let length = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or_default();
    body.len() >= length
}

fn reply(status: &str, body: &[u8]) -> Vec<u8> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut reply = head.into_bytes();
    reply.extend_from_slice(body);
    reply
}

async fn serve(reply: Vec<u8>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/fw/v1/analyze", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut socket, _peer) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        while !request_complete(&request) {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        socket.write_all(&reply).await.unwrap_or_default();
        request
    });
    Server { base, tasks }
}

fn poster() -> HttpPoster {
    HttpPoster {
        client: ClientBuilder::new().no_proxy().build().unwrap(),
    }
}

#[tokio::test]
async fn aim_guardrail_http_poster_sends_the_request_and_returns_the_status_and_body() {
    let headers = fixture(StatusCode::OK, "{}", &[])
        .guardrail
        .build_headers(AimHook::PreCall, Some("alias"), None, Some("call-1"))
        .unwrap();
    let mut server = serve(reply("503 Service Unavailable", b"oops")).await;
    let body = br#"{"messages":[]}"#;
    let response = poster()
        .post(&server.base, &headers, body, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.body, b"oops");
    let received = server.tasks.join_next().await.unwrap().unwrap();
    let request = String::from_utf8(received).unwrap();
    assert!(request.starts_with("POST /fw/v1/analyze HTTP/1.1\r\n"));
    assert!(request.contains("authorization: Bearer aim-key\r\n"));
    assert!(request.contains("x-aim-call-id: call-1\r\n"));
    assert!(request.contains("x-aim-gateway-key-alias: alias\r\n"));
    assert!(request.ends_with(r#"{"messages":[]}"#));
}

#[rstest]
#[case::small(4, true)]
#[case::just_inside(MAX_RESPONSE_BYTES.saturating_sub(1), true)]
#[case::at_the_limit(MAX_RESPONSE_BYTES, true)]
#[case::just_outside(MAX_RESPONSE_BYTES.saturating_add(1), false)]
#[tokio::test]
async fn aim_guardrail_http_poster_limits_the_response_size(
    #[case] size: usize,
    #[case] accepted: bool,
) {
    let server = serve(reply("200 OK", &vec![b'x'; size])).await;
    let result = poster()
        .post(&server.base, &HeaderMap::new(), b"{}", None)
        .await;
    if accepted {
        assert_eq!(result.unwrap().body.len(), size);
    } else {
        assert!(
            matches!(result, Err(AimError::ResponseTooLarge(limit)) if limit == MAX_RESPONSE_BYTES)
        );
    }
}

#[tokio::test]
async fn aim_guardrail_http_poster_reports_a_timeout_apart_from_a_failed_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fw/v1/analyze", listener.local_addr().unwrap());
    let silent = poster()
        .post(
            &url,
            &HeaderMap::new(),
            b"{}",
            Some(Duration::from_millis(100)),
        )
        .await;
    assert!(matches!(silent, Err(AimError::Timeout(_))));
    drop(listener);
    let refused = poster()
        .post(&url, &HeaderMap::new(), b"{}", Some(Duration::from_secs(5)))
        .await;
    assert!(matches!(refused, Err(AimError::Request(_))));
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
