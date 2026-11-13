#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::presidio_actions::request_data::{RequestData, pii_tokens};
use crate::presidio_actions::{AnonymizerSettings, PresidioAnonymizer};
use crate::presidio_analysis::wire::AnalyzeResponseItem;
use crate::presidio_analysis::{AnalyzerSettings, DEFAULT_CHUNK_SIZE_BYTES, PresidioAnalyzer};

pub(super) type Reply = (StatusCode, &'static str, String);

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SeenAnalyzeRequest {
    pub(super) text: String,
    #[serde(default)]
    pub(super) language: Option<String>,
    #[serde(default)]
    pub(super) entities: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SeenAnonymizeRequest {
    pub(super) text: String,
    pub(super) analyzer_results: Vec<Box<RawValue>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SeenAnalyzeItem {
    pub(super) entity_type: Option<String>,
    pub(super) start: Option<usize>,
    pub(super) end: Option<usize>,
    pub(super) score: Option<f64>,
    #[serde(default)]
    pub(super) analysis_explanation: Option<Box<RawValue>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct LoggedRecord {
    pub(super) guardrail_name: String,
    pub(super) guardrail_provider: String,
    pub(super) guardrail_response: Box<RawValue>,
    pub(super) guardrail_status: String,
    pub(super) start_time: f64,
    pub(super) end_time: f64,
    pub(super) duration: f64,
    pub(super) masked_entity_count: BTreeMap<String, usize>,
}

type Responder = Arc<dyn Fn(&[u8]) -> Reply + Send + Sync>;

#[derive(Clone)]
struct MockState {
    respond: Responder,
    delay: Duration,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}

pub(super) struct MockServer {
    pub(super) base: String,
    pub(super) requests: Arc<Mutex<Vec<Vec<u8>>>>,
    _server: JoinSet<io::Result<()>>,
}

async fn answer(State(state): State<MockState>, bytes: Bytes) -> Response {
    sleep(state.delay).await;
    let (status, content_type, body) = (state.respond)(&bytes);
    state.requests.lock().unwrap().push(bytes.to_vec());
    (status, [(header::CONTENT_TYPE, content_type)], body).into_response()
}

async fn spawn<F>(route: &str, respond: F, delay: Duration) -> MockServer
where
    F: Fn(&[u8]) -> Reply + Send + Sync + 'static,
{
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = MockState {
        respond: Arc::new(respond),
        delay,
        requests: Arc::clone(&requests),
    };
    let router = Router::new().route(route, post(answer)).with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    MockServer {
        base,
        requests,
        _server: server,
    }
}

pub(super) async fn spawn_analyzer<F>(respond: F, delay: Duration) -> MockServer
where
    F: Fn(&[u8]) -> Reply + Send + Sync + 'static,
{
    spawn("/analyze", respond, delay).await
}

pub(super) async fn spawn_anonymizer<F>(respond: F, delay: Duration) -> MockServer
where
    F: Fn(&[u8]) -> Reply + Send + Sync + 'static,
{
    spawn("/anonymize", respond, delay).await
}

fn parse_requests<Seen>(server: &MockServer) -> Vec<Seen>
where
    Seen: DeserializeOwned,
{
    server
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|body| serde_json::from_slice(body).unwrap())
        .collect()
}

pub(super) fn analyze_requests(server: &MockServer) -> Vec<SeenAnalyzeRequest> {
    parse_requests(server)
}

pub(super) fn anonymize_requests(server: &MockServer) -> Vec<SeenAnonymizeRequest> {
    parse_requests(server)
}

pub(super) fn seen_item(raw: &RawValue) -> SeenAnalyzeItem {
    serde_json::from_str(raw.get()).unwrap()
}

pub(super) fn json_reply(body: &str) -> Reply {
    (StatusCode::OK, "application/json", body.to_owned())
}

pub(super) fn json_reply_with(status: StatusCode, body: &str) -> Reply {
    (status, "application/json", body.to_owned())
}

pub(super) fn plain_reply(status: StatusCode, body: &str) -> Reply {
    (status, "text/plain", body.to_owned())
}

pub(super) fn fixed_clock() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

pub(super) fn settings(analyzer_base: &str) -> AnalyzerSettings {
    AnalyzerSettings {
        guardrail_name: "presidio-global".to_owned(),
        analyzer_api_base: format!("{analyzer_base}/"),
        language: "en".to_owned(),
        ad_hoc_recognizers: None,
        pii_entities: Vec::new(),
        output_parse_pii: false,
        apply_to_output: false,
        score_thresholds: BTreeMap::new(),
        entities_deny_list: Vec::new(),
        chunk_size_bytes: DEFAULT_CHUNK_SIZE_BYTES,
        timeout: Duration::from_secs(5),
        mock_redacted_text: None,
    }
}

pub(super) fn analyzer(analyzer_base: &str) -> PresidioAnalyzer {
    PresidioAnalyzer {
        settings: settings(analyzer_base),
        client: reqwest::Client::default(),
        chunk_semaphores: Mutex::default(),
    }
}

pub(super) fn anonymizer(anonymizer_base: &str) -> PresidioAnonymizer {
    let settings = AnonymizerSettings {
        anonymizer_api_base: format!("{anonymizer_base}/"),
        timeout: Duration::from_secs(5),
    };
    let client = reqwest::Client::default();
    PresidioAnonymizer {
        settings,
        client,
        clock: fixed_clock,
    }
}

pub(super) fn item(entity_type: &str, start: usize, end: usize) -> AnalyzeResponseItem {
    scored_item(entity_type, start, end, 1.0_f64)
}

pub(super) fn scored_item(
    entity_type: &str,
    start: usize,
    end: usize,
    score: f64,
) -> AnalyzeResponseItem {
    AnalyzeResponseItem {
        entity_type: Some(entity_type.to_owned()),
        start: Some(start),
        end: Some(end),
        score: Some(score),
        other: BTreeMap::new(),
    }
}

pub(super) fn request_data(json: &str) -> RequestData {
    serde_json::from_str(json).unwrap()
}

pub(super) fn tokens(request_data: &RequestData) -> IndexMap<String, String> {
    pii_tokens(request_data).unwrap()
}

pub(super) fn logged_records(request_data: &RequestData) -> Vec<LoggedRecord> {
    #[derive(Deserialize)]
    struct Bucket {
        #[serde(default)]
        standard_logging_guardrail_information: Vec<LoggedRecord>,
    }
    let key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Some(raw) = request_data.get(key) else {
        return Vec::new();
    };
    let bucket: Bucket = serde_json::from_str(raw.get()).unwrap();
    bucket.standard_logging_guardrail_information
}

pub(super) static NO_PARAMS: BTreeMap<String, GuardrailValue> = BTreeMap::new();
