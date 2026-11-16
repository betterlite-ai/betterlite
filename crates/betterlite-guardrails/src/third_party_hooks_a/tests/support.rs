use std::collections::{BTreeMap, VecDeque};
use std::future::IntoFuture as _;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::aim::settings::{AimGuardrail, AimParams};
use crate::aim::transport::{AimWebSocketConnector, HttpPoster};
use crate::lasso_masking::wire::ModelResponseShape;

pub(super) type Server = JoinSet<Result<(), IoError>>;

#[derive(Clone)]
pub(super) struct Call {
    pub(super) path: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
}

pub(super) type Captured = Arc<Mutex<Vec<Call>>>;

pub(super) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(super) fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

pub(super) fn list(items: &[GuardrailValue]) -> GuardrailValue {
    GuardrailValue::Array(items.to_vec())
}

pub(super) fn message(role: &str, content: &str) -> GuardrailValue {
    object(&[("role", text(role)), ("content", text(content))])
}

pub(super) fn chat_data(messages: &[GuardrailValue]) -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        ("litellm_call_id".to_owned(), text("call-1")),
        ("model".to_owned(), text("gpt-4o")),
        ("messages".to_owned(), list(messages)),
    ])
}

pub(super) fn captured_body(calls: &[Call], index: usize) -> GuardrailValue {
    serde_json::from_str(&calls[index].body).expect("captured body is JSON")
}

fn record(captured: &Captured, uri: &Uri, headers: &HeaderMap, body: &str) {
    let mut recorded = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(text) = value.to_str() {
            recorded.insert(name.as_str().to_owned(), text.to_owned());
        }
    }
    captured.lock().expect("captured").push(Call {
        path: uri.to_string(),
        headers: recorded,
        body: body.to_owned(),
    });
}

#[derive(Clone)]
struct MockState {
    captured: Captured,
    answers: Arc<Mutex<VecDeque<(StatusCode, String)>>>,
    aim: Option<AimMode>,
    held: Option<Arc<Semaphore>>,
}

async fn listen() -> (TcpListener, String, Captured) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    (listener, base, Arc::new(Mutex::new(Vec::new())))
}

fn spawn_server(listener: TcpListener, router: Router) -> Server {
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    server
}

async fn mock(
    route: &str,
    aim: Option<AimMode>,
    answers: Vec<(StatusCode, String)>,
    held: Option<Arc<Semaphore>>,
) -> (String, Server, Captured) {
    let (listener, base, captured) = listen().await;
    let state = MockState {
        captured: Arc::clone(&captured),
        answers: Arc::new(Mutex::new(VecDeque::from(answers))),
        aim,
        held,
    };
    let router = Router::new().route(route, post(handle)).with_state(state);
    (base, spawn_server(listener, router), captured)
}

pub(super) async fn serve(
    route: &str,
    answers: Vec<(StatusCode, String)>,
) -> (String, Server, Captured) {
    mock(route, None, answers, None).await
}

// an ingest_data=true request waits for a permit, so a caller that awaited the send would hang
pub(super) async fn serve_holding_ingest(
    route: &str,
    answers: Vec<(StatusCode, String)>,
    held: &Arc<Semaphore>,
) -> (String, Server, Captured) {
    mock(route, None, answers, Some(Arc::clone(held))).await
}

async fn handle(
    State(state): State<MockState>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, String) {
    record(&state.captured, &uri, &headers, &body);
    if let Some(held) = &state.held
        && uri
            .query()
            .is_some_and(|query| query.contains("ingest_data=true"))
    {
        drop(held.acquire().await.expect("permit"));
    }
    if let Some(mode) = state.aim {
        let answer = aim_answer(mode, &body);
        return (
            StatusCode::OK,
            serde_json::to_string(&answer).expect("aim answer"),
        );
    }
    state
        .answers
        .lock()
        .expect("answers")
        .pop_front()
        .unwrap_or((StatusCode::OK, String::new()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AimMode {
    Monitor,
    Redact,
    Block,
    BlockSilently,
}

#[derive(Deserialize)]
struct AimRequest {
    messages: Vec<MessageRow>,
}

#[derive(Clone, Deserialize, Serialize)]
struct MessageRow {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AimAction {
    action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detection_message: Option<String>,
}

#[derive(Serialize)]
struct RedactedChat {
    all_redacted_messages: Vec<MessageRow>,
}

#[derive(Serialize)]
struct AimAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    required_action: Option<AimAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redacted_chat: Option<RedactedChat>,
}

pub(super) async fn serve_aim(mode: AimMode) -> (String, Server, Captured) {
    mock("/fw/v1/analyze", Some(mode), Vec::new(), None).await
}

fn aim_answer(mode: AimMode, body: &str) -> AimAnswer {
    match mode {
        AimMode::Monitor => plain_answer("monitor_action", None),
        AimMode::Block => plain_answer("block_action", Some("AIM says no")),
        AimMode::BlockSilently => plain_answer("block_action", None),
        AimMode::Redact => {
            let request: AimRequest = serde_json::from_str(body).expect("aim request body");
            redacted_answer(&request.messages)
        }
    }
}

fn aim_action(action_type: &str, detection_message: Option<&str>) -> AimAction {
    AimAction {
        action_type: action_type.to_owned(),
        detection_message: detection_message.map(str::to_owned),
    }
}

fn plain_answer(action_type: &str, detection_message: Option<&str>) -> AimAnswer {
    AimAnswer {
        required_action: Some(aim_action(action_type, detection_message)),
        redacted_chat: None,
    }
}

// the mock redacts the last inspected message, so one choice's output comes back rewritten
fn redacted_answer(messages: &[MessageRow]) -> AimAnswer {
    let last = messages.len().saturating_sub(1);
    let redacted = messages
        .iter()
        .enumerate()
        .map(|(index, message)| MessageRow {
            role: message.role.clone(),
            content: if index == last {
                format!("redacted:{}", message.content)
            } else {
                message.content.clone()
            },
        })
        .collect();
    AimAnswer {
        required_action: Some(aim_action("anonymize_action", None)),
        redacted_chat: Some(RedactedChat {
            all_redacted_messages: redacted,
        }),
    }
}

pub(super) fn aim_guardrail(base: &str, inspect_embeddings: bool) -> AimGuardrail {
    AimGuardrail::new(
        AimParams {
            guardrail_name: Some("aim".to_owned()),
            api_key: Some(SecretString::from("aim-key")),
            api_base: Some(base.to_owned()),
            inspect_embeddings: Some(inspect_embeddings),
            litellm_version: None,
        },
        &BTreeMap::<String, String>::new(),
        Arc::new(HttpPoster {
            client: reqwest::Client::default(),
        }),
        Arc::new(AimWebSocketConnector),
        Some(Duration::from_secs(5)),
    )
    .expect("guardrail")
}

pub(super) fn chat_response(contents: &[&str]) -> ModelResponseShape {
    let choices: Vec<GuardrailValue> = contents
        .iter()
        .map(|content| object(&[("message", message("assistant", content))]))
        .collect();
    let raw =
        serde_json::to_string(&object(&[("choices", list(&choices))])).expect("response json");
    serde_json::from_str(&raw).expect("response")
}

pub(super) fn content_of(response: &ModelResponseShape, index: usize) -> Option<String> {
    let message = response.choices[index].message.as_ref()?;
    message.content.clone()
}
