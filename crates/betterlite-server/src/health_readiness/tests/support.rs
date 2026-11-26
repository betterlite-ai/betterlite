use std::collections::VecDeque;
use std::future::pending;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderName, Method, Request, StatusCode};
use axum::response::Response;
use betterlite_config::sections::aiohttp_transport_toggle::Settings as AiohttpSettings;
use betterlite_config::sections::env_credential_login::Settings as EnvLoginSettings;
use betterlite_config::sections::health_readiness::Settings as HealthSettings;
use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use betterlite_db::database_faults::{DbError, ErrorType};
use serde::de::DeserializeOwned;
use tokio::time::Instant;
use tower::ServiceExt as _;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::health_readiness::callback_listing::{CallbackNameSource, CallbackNames};
use crate::health_readiness::db_readiness::{DatabaseProbe, DbReadiness, StallTracker};
use crate::health_readiness::readiness_details::{
    CacheReport, ReadinessSourceError, ReadinessSources,
};
use crate::health_readiness::readiness_routes::{
    AdmissionCounter, AdmissionCounters, ReadinessPorts, ReadinessState, readiness_routes,
};
use crate::health_readiness::shutdown_state::InFlightCounter;

pub(crate) type Reply = (StatusCode, HeaderMap, Vec<u8>);

#[derive(Debug)]
pub(crate) struct FakeInFlight {
    pub(crate) count: AtomicU64,
}

impl FakeInFlight {
    pub(crate) const fn new(count: u64) -> Self {
        Self {
            count: AtomicU64::new(count),
        }
    }
}

impl Default for FakeInFlight {
    fn default() -> Self {
        Self::new(0)
    }
}

impl InFlightCounter for FakeInFlight {
    fn in_flight_requests(&self) -> u64 {
        self.count.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FakeAdmission {
    pub(crate) stats: AdmissionCounters,
}

impl AdmissionCounter for FakeAdmission {
    fn counters(&self) -> AdmissionCounters {
        self.stats
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FakeStall {
    pub(crate) stalled: bool,
}

impl StallTracker for FakeStall {
    fn stalled_within_window(&self, _window_seconds: f64) -> bool {
        self.stalled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeAnswer {
    Ok,
    Fail(ErrorType),
    Hang,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconnectAnswer {
    Report(bool),
    Fail(ErrorType),
    Hang,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerCensus {
    Failed,
    Workers(u32),
}

pub(crate) struct FakeProbe {
    pub(crate) configured: bool,
    pub(crate) census: WorkerCensus,
    pub(crate) health_calls: AtomicU64,
    pub(crate) last_health_call: Mutex<Option<Instant>>,
    pub(crate) reconnect_calls: Mutex<Vec<(String, f64, f64)>>,
    answers: Mutex<VecDeque<ProbeAnswer>>,
    reconnects: Mutex<VecDeque<ReconnectAnswer>>,
}

impl FakeProbe {
    pub(crate) fn unconfigured() -> Self {
        Self {
            configured: false,
            census: WorkerCensus::Workers(1),
            health_calls: AtomicU64::new(0),
            last_health_call: Mutex::new(None),
            reconnect_calls: Mutex::new(Vec::new()),
            answers: Mutex::new(VecDeque::new()),
            reconnects: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn scripted(answers: Vec<ProbeAnswer>) -> Self {
        let mut probe = Self::unconfigured();
        probe.configured = true;
        *probe.answers.lock().unwrap_or_else(PoisonError::into_inner) = VecDeque::from(answers);
        probe
    }

    pub(crate) fn healthy() -> Self {
        Self::scripted(Vec::new())
    }

    pub(crate) fn with_reconnects(mut self, reconnects: Vec<ReconnectAnswer>) -> Self {
        *self
            .reconnects
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = VecDeque::from(reconnects);
        self
    }

    pub(crate) fn reconnect_requests(&self) -> Vec<(String, f64, f64)> {
        self.reconnect_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next_answer(&self) -> ProbeAnswer {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ProbeAnswer::Ok)
    }

    fn next_reconnect(&self) -> ReconnectAnswer {
        self.reconnects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ReconnectAnswer::Report(true))
    }
}

impl Default for FakeProbe {
    fn default() -> Self {
        Self::unconfigured()
    }
}

pub(crate) fn db_error(error_type: ErrorType) -> DbError {
    DbError {
        error_type,
        message: String::new(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

#[async_trait]
impl DatabaseProbe for FakeProbe {
    fn configured(&self) -> bool {
        self.configured
    }

    async fn health_check(&self) -> Result<(), DbError> {
        self.health_calls.fetch_add(1, Ordering::AcqRel);
        *self
            .last_health_call
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        match self.next_answer() {
            ProbeAnswer::Ok => Ok(()),
            ProbeAnswer::Fail(error_type) => Err(db_error(error_type)),
            ProbeAnswer::Hang => pending::<Result<(), DbError>>().await,
        }
    }

    async fn attempt_reconnect(
        &self,
        reason: &str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Result<bool, DbError> {
        self.reconnect_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((reason.to_owned(), timeout_seconds, lock_timeout_seconds));
        match self.next_reconnect() {
            ReconnectAnswer::Report(attempted) => Ok(attempted),
            ReconnectAnswer::Fail(error_type) => Err(db_error(error_type)),
            ReconnectAnswer::Hang => pending::<Result<bool, DbError>>().await,
        }
    }

    async fn live_worker_count(&self) -> Option<u32> {
        match self.census {
            WorkerCensus::Failed => None,
            WorkerCensus::Workers(count) => Some(count),
        }
    }
}

pub(crate) struct FakeSources {
    pub(crate) callbacks: Vec<String>,
    pub(crate) cache: Result<Option<CacheReport>, String>,
    pub(crate) redis: bool,
    pub(crate) sso: bool,
    pub(crate) level: String,
    pub(crate) version: String,
}

impl Default for FakeSources {
    fn default() -> Self {
        Self {
            callbacks: Vec::new(),
            cache: Ok(None),
            redis: false,
            sso: false,
            level: "INFO".to_owned(),
            version: "betterlite-test".to_owned(),
        }
    }
}

#[async_trait]
impl ReadinessSources for FakeSources {
    fn success_callback_names(&self) -> Vec<String> {
        self.callbacks.clone()
    }

    async fn cache_report(&self) -> Result<Option<CacheReport>, ReadinessSourceError> {
        self.cache
            .clone()
            .map_err(|message| ReadinessSourceError::Unavailable { message })
    }

    fn redis_configured(&self) -> bool {
        self.redis
    }

    fn sso_provider_ready(&self) -> bool {
        self.sso
    }

    fn log_level(&self) -> String {
        self.level.clone()
    }

    fn version(&self) -> String {
        self.version.clone()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FakeCallbacks {
    pub(crate) names: CallbackNames,
}

impl CallbackNameSource for FakeCallbacks {
    fn callback_names(&self) -> CallbackNames {
        self.names.clone()
    }
}

pub(crate) fn db_readiness(
    probe: &Arc<FakeProbe>,
    stall: &Arc<FakeStall>,
    availability: AllowRequestsOnOutage,
    settings: &HealthSettings,
) -> DbReadiness {
    DbReadiness::new(
        Arc::clone(probe),
        Arc::clone(stall),
        availability,
        settings.db_lookup_stall_window_seconds,
        settings.disable_no_redis_warning,
    )
}

#[derive(Default)]
pub(crate) struct World {
    pub(crate) in_flight: Arc<FakeInFlight>,
    pub(crate) admission: Arc<FakeAdmission>,
    pub(crate) probe: Arc<FakeProbe>,
    pub(crate) stall: Arc<FakeStall>,
    pub(crate) sources: Arc<FakeSources>,
    pub(crate) callbacks: Arc<FakeCallbacks>,
    pub(crate) settings: HealthSettings,
}

impl World {
    pub(crate) fn router(&self, availability: AllowRequestsOnOutage) -> Router {
        let db = Arc::new(db_readiness(
            &self.probe,
            &self.stall,
            availability,
            &self.settings,
        ));
        let in_flight: Arc<dyn InFlightCounter> = Arc::clone(&self.in_flight);
        let admission: Arc<dyn AdmissionCounter> = Arc::clone(&self.admission);
        let sources: Arc<dyn ReadinessSources> = Arc::clone(&self.sources);
        let callbacks: Arc<dyn CallbackNameSource> = Arc::clone(&self.callbacks);
        let ports = ReadinessPorts {
            in_flight,
            admission,
            db,
            sources,
            callbacks,
        };
        readiness_routes(ReadinessState::new(
            ports,
            self.settings.clone(),
            EnvLoginSettings::default(),
            AiohttpSettings::default(),
        ))
    }
}

pub(crate) async fn send(
    router: &Router,
    method: Method,
    path: &str,
    header_value: Option<(&str, &str)>,
) -> Reply {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some((name, value)) = header_value {
        builder = builder.header(name, value);
    }
    let request = builder.body(Body::empty()).expect("the request builds");
    let response = Router::clone(router)
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    (status, headers, body.to_vec())
}

// the response is returned unread, so a body-held in-flight guard is still counting
pub(crate) async fn send_unread(router: &Router, method: Method, path: &str) -> Response {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("the request builds");
    Router::clone(router)
        .oneshot(request)
        .await
        .expect("the router answers")
}

pub(crate) fn parse<Parsed: DeserializeOwned>(body: &[u8]) -> Parsed {
    match serde_json::from_slice(body) {
        Ok(parsed) => parsed,
        Err(error) => panic!("the response body is not the documented JSON: {error}"),
    }
}

pub(crate) fn header_text(headers: &HeaderMap, name: &HeaderName) -> String {
    let value = headers.get(name).expect("the header is set");
    value.to_str().expect("the header is text").to_owned()
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the captured log lines are UTF-8")
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

pub(crate) fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
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
