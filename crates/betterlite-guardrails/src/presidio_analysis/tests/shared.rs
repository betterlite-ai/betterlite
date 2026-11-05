#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Result as IoResult, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::presidio_analysis::wire::AnalyzeResponseItem;
use crate::presidio_analysis::{
    AnalyzerSettings, DEFAULT_CHUNK_SIZE_BYTES, PresidioAnalyzer, RequestOverrides,
};

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SeenRequest {
    pub(super) text: String,
    #[serde(default)]
    pub(super) language: Option<String>,
    #[serde(default)]
    pub(super) entities: Option<Vec<String>>,
}

pub(super) type Reply = (StatusCode, &'static str, String);

type Responder = Arc<dyn Fn(&SeenRequest) -> Reply + Send + Sync>;

#[derive(Clone)]
struct MockState {
    respond: Responder,
    delay: Duration,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
    in_flight: Arc<AtomicUsize>,
    peak_in_flight: Arc<AtomicUsize>,
}

pub(super) struct MockAnalyzer {
    pub(super) base: String,
    pub(super) seen: Arc<Mutex<Vec<SeenRequest>>>,
    pub(super) peak_in_flight: Arc<AtomicUsize>,
    _server: JoinSet<IoResult<()>>,
}

async fn answer(State(state): State<MockState>, bytes: Bytes) -> Response {
    let request: SeenRequest = serde_json::from_slice(&bytes).unwrap();
    let now = state
        .in_flight
        .fetch_add(1, Ordering::SeqCst)
        .saturating_add(1);
    state.peak_in_flight.fetch_max(now, Ordering::SeqCst);
    sleep(state.delay).await;
    state.in_flight.fetch_sub(1, Ordering::SeqCst);
    let (status, content_type, body) = (state.respond)(&request);
    state.seen.lock().unwrap().push(request);
    (status, [(header::CONTENT_TYPE, content_type)], body).into_response()
}

pub(super) async fn spawn_mock<F>(respond: F, delay: Duration) -> MockAnalyzer
where
    F: Fn(&SeenRequest) -> Reply + Send + Sync + 'static,
{
    let seen = Arc::new(Mutex::new(Vec::new()));
    let peak_in_flight = Arc::new(AtomicUsize::new(0));
    let state = MockState {
        respond: Arc::new(respond),
        delay,
        seen: Arc::clone(&seen),
        in_flight: Arc::new(AtomicUsize::new(0)),
        peak_in_flight: Arc::clone(&peak_in_flight),
    };
    let router = Router::new()
        .route("/analyze", post(answer))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    MockAnalyzer {
        base,
        seen,
        peak_in_flight,
        _server: server,
    }
}

pub(super) fn settings(base: &str) -> AnalyzerSettings {
    AnalyzerSettings {
        guardrail_name: "presidio-guard".to_owned(),
        analyzer_api_base: format!("{base}/"),
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

pub(super) fn analyzer(settings: AnalyzerSettings) -> PresidioAnalyzer {
    PresidioAnalyzer {
        settings,
        client: reqwest::Client::default(),
        chunk_semaphores: Mutex::default(),
    }
}

pub(super) fn item(entity_type: &str, start: usize, end: usize, score: f64) -> AnalyzeResponseItem {
    AnalyzeResponseItem {
        entity_type: Some(entity_type.to_owned()),
        start: Some(start),
        end: Some(end),
        score: Some(score),
        other: BTreeMap::new(),
    }
}

pub(super) fn json_reply(body: &str) -> Reply {
    (StatusCode::OK, "application/json", body.to_owned())
}

pub(super) static NO_PARAMS: BTreeMap<String, GuardrailValue> = BTreeMap::new();

pub(super) static NO_OVERRIDES: RequestOverrides<'static> = RequestOverrides {
    language: None,
    dynamic_params: &NO_PARAMS,
};

// The three settings that make an invalid analyzer response a hard failure.
#[derive(Debug, Clone, Copy)]
pub(super) enum Trigger {
    PiiEntities,
    OutputParsePii,
    ApplyToOutput,
}

pub(super) fn closed_settings(base: &str, trigger: Trigger) -> AnalyzerSettings {
    let mut configured = settings(base);
    match trigger {
        Trigger::PiiEntities => configured.pii_entities = vec!["PERSON".to_owned()],
        Trigger::OutputParsePii => configured.output_parse_pii = true,
        Trigger::ApplyToOutput => configured.apply_to_output = true,
    }
    configured
}

#[derive(Clone, Default)]
pub(super) struct Captured(pub(super) Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
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

// Reports "SECRET" as a PERSON at its position inside the chunk it received.
pub(super) fn find_secret(request: &SeenRequest) -> Reply {
    request.text.find("SECRET").map_or_else(
        || json_reply("[]"),
        |start| {
            json_reply(&format!(
                r#"[{{"entity_type":"PERSON","start":{start},"end":{},"score":0.9}}]"#,
                start.saturating_add(6)
            ))
        },
    )
}

pub(super) fn text_with_secret_at_33() -> String {
    format!("{}SECRET{}", "a".repeat(33), "b".repeat(11))
}
