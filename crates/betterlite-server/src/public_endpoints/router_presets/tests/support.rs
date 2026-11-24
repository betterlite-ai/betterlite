use std::collections::VecDeque;
use std::io;
use std::str::from_utf8;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use betterlite_config::sections::public_router_presets::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::yield_now;
use tower::ServiceExt as _;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::super::catalog::{BUNDLED_CATALOG, PresetCatalog, decode_catalog};
use super::super::router_presets_routes;
use super::super::service::AutorouterPresetService;
use crate::roi_github::transport::Transport;

pub(super) const PRESETS_URL: &str = "http://presets.test/catalog.json";
pub(super) const MAX_BODY: usize = 1024 * 1024;

pub(super) const REMOTE_CATALOG: &str = r#"{
  "custom": {
    "label": "Custom",
    "description": "A catalog served by the remote.",
    "badge": "gold",
    "complexity_router_config": {
      "tiers": {"SIMPLE": ["small"], "MEDIUM": ["medium"], "COMPLEX": ["large"], "REASONING": ["thinker"]},
      "classifier_type": "heuristic"
    }
  }
}"#;

pub(super) enum Answer {
    Reply { status: u16, body: Vec<u8> },
    Moved { status: u16, location: String },
    Closed,
}

#[derive(Clone, Debug)]
pub(super) struct Seen {
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) deadline: Duration,
}

#[derive(Clone, Default)]
pub(super) struct Script {
    replies: Arc<Mutex<VecDeque<Answer>>>,
    calls: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Script {
    pub(super) fn new(replies: Vec<Answer>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            calls: Arc::new(AtomicUsize::new(0)),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(super) fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Script {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                method: request.method,
                url: request.url.to_string(),
                deadline: request.deadline,
            });
        yield_now().await;
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match next {
            Some(Answer::Reply { status, body }) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Some(Answer::Moved { status, location }) => {
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::LOCATION,
                    HeaderValue::try_from(location)
                        .expect("the scripted Location is a header value"),
                );
                Ok(UpstreamResponse {
                    status,
                    headers,
                    body: Vec::new(),
                })
            }
            Some(Answer::Closed) | None => Err(TransportError::Closed),
        }
    }
}

pub(super) fn settings(url: &str, local_only: bool) -> Settings {
    Settings {
        local_only,
        url: url.to_owned(),
    }
}

pub(super) fn app(script: &Script, settings: Settings) -> Router {
    router_presets_routes(Arc::new(AutorouterPresetService::new(
        script.clone(),
        settings,
    )))
}

pub(super) fn app_with_bundled(script: &Script, bundled: &'static str, local_only: bool) -> Router {
    let service = Arc::new(AutorouterPresetService {
        client: script.clone(),
        url: PRESETS_URL.to_owned(),
        local_only,
        bundled,
        cache: AsyncMutex::new(None),
        fetch_lock: AsyncMutex::new(()),
    });
    router_presets_routes(service)
}

pub(super) fn bundled_catalog() -> PresetCatalog {
    decode_catalog(BUNDLED_CATALOG.as_bytes()).expect("the bundled catalog should decode")
}

pub(super) fn parsed(text: &str) -> PresetCatalog {
    serde_json::from_str(text).expect("the response should be a preset catalog")
}

pub(super) async fn get(app: Router, path: &str) -> (StatusCode, String, String) {
    send(app, Method::GET, path).await
}

pub(super) async fn send(app: Router, method: Method, path: &str) -> (StatusCode, String, String) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("the request should build");
    let response = app
        .oneshot(request)
        .await
        .expect("the router should answer");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = to_bytes(response.into_body(), MAX_BODY)
        .await
        .expect("the body should be readable");
    let text = String::from_utf8(body.to_vec()).expect("the body should be UTF-8");
    (status, content_type, text)
}

pub(super) fn reply(status: u16, body: &str) -> Answer {
    Answer::Reply {
        status,
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn moved(status: u16, location: &str) -> Answer {
    Answer::Moved {
        status,
        location: location.to_owned(),
    }
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
