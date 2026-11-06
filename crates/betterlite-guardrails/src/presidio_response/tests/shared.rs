#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::sync::{Barrier, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::presidio_actions::request_data::RequestData;
use crate::presidio_actions::{AnonymizerSettings, PresidioAnonymizer};
use crate::presidio_analysis::wire::{MockRedactedItem, MockRedactedText};
use crate::presidio_analysis::{AnalyzerSettings, DEFAULT_CHUNK_SIZE_BYTES, PresidioAnalyzer};
use crate::presidio_response::HookContext;
use crate::presidio_response::wire::{ChatMessage, Content, ContentItem, ResponseBody};

pub(super) static NO_ENTITIES: BTreeMap<String, PiiAction> = BTreeMap::new();
pub(super) static NO_PARAMS: BTreeMap<String, GuardrailValue> = BTreeMap::new();

const SLOW_TEXT: &str = "Slow";
const DELAYED_TEXT: &str = "Alice";
const DELAYED_REPLY: Duration = Duration::from_millis(100);
const BARRIER_TIMEOUT: Duration = Duration::from_secs(2);
const LOGGING_KEY: &str = "standard_logging_guardrail_information";

pub(super) fn request_data(json: &str) -> RequestData {
    serde_json::from_str(json).expect("request data")
}

pub(super) fn response_body(json: &str) -> ResponseBody {
    let raw: Box<RawValue> = serde_json::from_str(json).expect("response JSON");
    ResponseBody::from_raw(raw).expect("response body")
}

pub(super) fn analyzer(api_base: &str, masked: Option<&str>) -> PresidioAnalyzer {
    PresidioAnalyzer {
        settings: AnalyzerSettings {
            guardrail_name: "presidio-global".to_owned(),
            analyzer_api_base: format!("{api_base}/"),
            language: "en".to_owned(),
            ad_hoc_recognizers: None,
            pii_entities: Vec::new(),
            output_parse_pii: false,
            apply_to_output: false,
            score_thresholds: BTreeMap::new(),
            entities_deny_list: Vec::new(),
            chunk_size_bytes: DEFAULT_CHUNK_SIZE_BYTES,
            timeout: Duration::from_secs(5),
            mock_redacted_text: masked.map(mock_text),
        },
        client: reqwest::Client::default(),
        chunk_semaphores: Mutex::default(),
    }
}

fn mock_text(text: &str) -> MockRedactedText {
    MockRedactedText {
        text: text.to_owned(),
        items: vec![MockRedactedItem {
            entity_type: Some("PERSON".to_owned()),
        }],
    }
}

pub(super) fn anonymizer(api_base: &str) -> PresidioAnonymizer {
    PresidioAnonymizer {
        settings: AnonymizerSettings {
            anonymizer_api_base: format!("{api_base}/"),
            timeout: Duration::from_secs(5),
        },
        client: reqwest::Client::default(),
        clock: fixed_clock,
    }
}

fn fixed_clock() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("fixed clock")
}

pub(super) fn context<'context>(
    analyzer: &'context PresidioAnalyzer,
    anonymizer: &'context PresidioAnonymizer,
) -> HookContext<'context> {
    HookContext {
        analyzer,
        anonymizer,
        pii_entities_config: &NO_ENTITIES,
        dynamic_params: &NO_PARAMS,
        global_output_parse_pii: false,
    }
}

pub(super) fn messages(request: &RequestData) -> Vec<ChatMessage> {
    let raw = request.get("messages").expect("messages field");
    serde_json::from_str(raw.get()).expect("messages JSON")
}

pub(super) fn content_text(content: Option<&Content>) -> Option<String> {
    match content? {
        Content::Text(text) => Some(text.clone()),
        Content::Blocks(_) | Content::Other(_) => None,
    }
}

pub(super) fn chat_content(response: &ResponseBody) -> Option<String> {
    let ResponseBody::Chat(chat) = response else {
        return None;
    };
    let choice = chat.choices.as_ref()?.first()?;
    let message = choice.message.as_ref()?;
    content_text(message.content.as_ref())
}

pub(super) fn anthropic_texts(response: &ResponseBody) -> Vec<String> {
    let ResponseBody::Anthropic(message) = response else {
        return Vec::new();
    };
    message
        .content
        .iter()
        .flatten()
        .filter_map(|item| match item {
            ContentItem::Object(part) if part.kind.as_deref() == Some("text") => part.text.clone(),
            ContentItem::Object(_) | ContentItem::Other(_) => None,
        })
        .collect()
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture_logs() -> (Arc<Mutex<Vec<u8>>>, DefaultGuard) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Sink(Arc::clone(&captured));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_writer(move || sink.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn logged_text(captured: &Mutex<Vec<u8>>) -> String {
    let bytes = captured
        .lock()
        .map_or_else(|_poisoned| Vec::new(), |log| log.clone());
    String::from_utf8(bytes).expect("captured log is UTF-8")
}

#[derive(Clone, Deserialize)]
pub(super) struct SeenAnalyze {
    pub(super) text: String,
    pub(super) language: Option<String>,
}

#[derive(Deserialize)]
struct SeenAnonymize {
    text: String,
}

#[derive(Clone)]
struct MockState {
    barrier: Arc<Barrier>,
    seen: Arc<Mutex<Vec<SeenAnalyze>>>,
    started: Arc<Semaphore>,
    release: Arc<Semaphore>,
}

pub(super) struct MockServer {
    pub(super) base: String,
    pub(super) seen: Arc<Mutex<Vec<SeenAnalyze>>>,
    pub(super) started: Arc<Semaphore>,
    pub(super) release: Arc<Semaphore>,
    _server: JoinSet<io::Result<()>>,
}

fn entity_for(text: &str) -> &'static str {
    match text {
        "Bobby" => "EMAIL_ADDRESS",
        "Carla" => "PHONE_NUMBER",
        _ => "PERSON",
    }
}

async fn analyze(State(state): State<MockState>, body: Bytes) -> Response {
    let Ok(seen) = serde_json::from_slice::<SeenAnalyze>(&body) else {
        return (StatusCode::BAD_REQUEST, "missing text").into_response();
    };
    let text = seen.text.clone();
    state.seen.lock().expect("seen requests").push(seen);
    if timeout(BARRIER_TIMEOUT, state.barrier.wait())
        .await
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "the checks ran one at a time",
        )
            .into_response();
    }
    if text == SLOW_TEXT {
        state.started.add_permits(1);
        if state.release.acquire().await.is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "released").into_response();
        }
    }
    let detection = format!(
        r#"[{{"entity_type":"{}","start":0,"end":5,"score":0.9}}]"#,
        entity_for(&text)
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        detection,
    )
        .into_response()
}

async fn anonymize(body: Bytes) -> Response {
    let Ok(seen) = serde_json::from_slice::<SeenAnonymize>(&body) else {
        return (StatusCode::BAD_REQUEST, "missing text").into_response();
    };
    if seen.text == DELAYED_TEXT {
        sleep(DELAYED_REPLY).await;
    }
    let reply = format!(
        r#"{{"text":"MASKED-{}","items":[{{"entity_type":"{}"}}]}}"#,
        seen.text,
        entity_for(&seen.text)
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        reply,
    )
        .into_response()
}

pub(super) async fn spawn_mock(concurrent: usize) -> MockServer {
    let state = MockState {
        barrier: Arc::new(Barrier::new(concurrent)),
        seen: Arc::new(Mutex::new(Vec::new())),
        started: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
    };
    let seen = Arc::clone(&state.seen);
    let started = Arc::clone(&state.started);
    let release = Arc::clone(&state.release);
    let router = Router::new()
        .route("/analyze", post(analyze))
        .route("/anonymize", post(anonymize))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    MockServer {
        base,
        seen,
        started,
        release,
        _server: server,
    }
}

fn log_entries_json(request: &RequestData) -> Option<String> {
    let bucket = request
        .get("litellm_metadata")
        .or_else(|| request.get("metadata"))?;
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(bucket.get()).ok()?;
    let entries = metadata.get(LOGGING_KEY)?;
    Some(entries.get().to_owned())
}

pub(super) fn logged_count(request: &RequestData) -> usize {
    log_entries_json(request).map_or(0, |json| {
        serde_json::from_str::<Vec<Box<RawValue>>>(&json).map_or(0, |entries| entries.len())
    })
}

#[derive(Deserialize)]
struct LoggedRecord {
    #[serde(default)]
    guardrail_name: Option<String>,
    #[serde(default)]
    guardrail_response: Vec<LoggedItem>,
}

#[derive(Deserialize)]
struct LoggedItem {
    #[serde(default)]
    entity_type: Option<String>,
}

fn logged_records(request: &RequestData) -> Vec<LoggedRecord> {
    let Some(json) = log_entries_json(request) else {
        return Vec::new();
    };
    serde_json::from_str(&json).expect("logged records")
}

pub(super) fn logged_names(request: &RequestData) -> Vec<String> {
    logged_records(request)
        .into_iter()
        .map(|record| record.guardrail_name.unwrap_or_default())
        .collect()
}

pub(super) fn logged_entity_types(request: &RequestData) -> Vec<String> {
    logged_records(request)
        .into_iter()
        .map(|record| {
            record
                .guardrail_response
                .first()
                .and_then(|item| item.entity_type.clone())
                .unwrap_or_default()
        })
        .collect()
}
