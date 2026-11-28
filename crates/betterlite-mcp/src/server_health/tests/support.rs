use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, pending};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::openapi_tools::error::OpenApiError;
use crate::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use crate::server_health::ports::{
    AllowedServers, Clock, EnvVarCheck, ProbeClient, ProbeError, SessionError, SessionProbe,
    SpecLoader,
};
use crate::server_health::service::{HealthDeps, HealthService};
use crate::server_health::settings::HealthSettings;
use crate::server_record::MCPServer;

pub(super) const NOW_TEXT: &str = "2026-01-02T03:04:05Z";

pub(super) type TestService =
    HealthService<FakeProbe, FakeSession, FakeEnvVars, FakeAllowed, FakeClock, FakeLoader>;

pub(super) fn record(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

pub(super) fn server_json(server_id: &str, transport: &str, extra: &str) -> String {
    format!(
        r#"{{"server_id": "{server_id}", "name": "Name {server_id}", "transport": "{transport}"{extra}}}"#
    )
}

pub(super) fn server_ids(rows: &[McpServerTable]) -> Vec<String> {
    rows.iter().map(|row| row.server_id.clone()).collect()
}

pub(super) fn timestamp_text(value: Option<RowTimestamp>) -> Option<String> {
    let stamp = value?;
    let encoded = serde_json::to_string(&stamp).unwrap();
    Some(serde_json::from_str(&encoded).unwrap())
}

pub(super) struct Rig {
    pub(super) service: TestService,
    pub(super) probe: Arc<ProbeLog>,
    pub(super) session: Arc<SessionLog>,
    pub(super) env_vars: Arc<EnvLog>,
    pub(super) allowed: Arc<AllowedLog>,
    pub(super) clock: FakeClock,
    pub(super) loader: Arc<LoaderLog>,
    pub(super) cancel: CancellationToken,
}

pub(super) fn rig(config: &[&str], db: &[&str]) -> Rig {
    rig_with(config, db, HealthSettings::default())
}

pub(super) fn rig_with(config: &[&str], db: &[&str], settings: HealthSettings) -> Rig {
    let registry = ServerRegistry::new(registry_settings(), records(config), records(db));
    let allowed = Arc::new(AllowedLog::default());
    allowed.set_ids(
        registry
            .get_registry()
            .into_iter()
            .map(|server| server.fields.server_id.clone())
            .collect(),
    );
    let probe = Arc::new(ProbeLog::default());
    let session = Arc::new(SessionLog::default());
    let env_vars = Arc::new(EnvLog::default());
    let loader = Arc::new(LoaderLog::default());
    let clock = FakeClock::new();
    let cancel = CancellationToken::new();
    let registry: SharedRegistry = Arc::new(RwLock::new(registry));
    let deps = HealthDeps {
        registry,
        probe: FakeProbe(Arc::clone(&probe)),
        sessions: FakeSession(Arc::clone(&session)),
        env_vars: FakeEnvVars(Arc::clone(&env_vars)),
        allowed: FakeAllowed(Arc::clone(&allowed)),
        clock: clock.clone(),
        specs: FakeLoader(Arc::clone(&loader)),
        cancel: cancel.clone(),
    };
    Rig {
        service: HealthService::new(deps, settings),
        probe,
        session,
        env_vars,
        allowed,
        clock,
        loader,
        cancel,
    }
}

pub(super) fn fake_loader() -> (FakeLoader, Arc<LoaderLog>) {
    let log = Arc::new(LoaderLog::default());
    (FakeLoader(Arc::clone(&log)), log)
}

const fn registry_settings() -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: true,
        internal_ip_ranges: None,
    }
}

fn records(entries: &[&str]) -> Vec<MCPServer> {
    entries.iter().map(|json| record(json)).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProbeCall {
    pub url: String,
    pub accept: String,
    pub timeout: Duration,
}

#[derive(Default)]
pub(super) struct ProbeLog {
    results: Mutex<VecDeque<Result<(), ProbeError>>>,
    calls: Mutex<Vec<ProbeCall>>,
}

impl ProbeLog {
    pub(super) fn script(&self, outcome: Result<(), ProbeError>) {
        self.results.lock().unwrap().push_back(outcome);
    }

    pub(super) fn calls(&self) -> Vec<ProbeCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[derive(Clone)]
pub(super) struct FakeProbe(Arc<ProbeLog>);

impl ProbeClient for FakeProbe {
    fn get_stream_status(
        &self,
        url: &str,
        accept: &str,
        timeout: Duration,
    ) -> impl Future<Output = Result<(), ProbeError>> + Send {
        let log = Arc::clone(&self.0);
        let call = ProbeCall {
            url: url.to_owned(),
            accept: accept.to_owned(),
            timeout,
        };
        async move {
            log.calls.lock().unwrap().push(call);
            log.results.lock().unwrap().pop_front().unwrap_or(Ok(()))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionCall {
    pub server_id: String,
    pub timeout: Duration,
    pub static_headers: bool,
}

#[derive(Default)]
pub(super) struct SessionLog {
    results: Mutex<VecDeque<Result<Option<String>, SessionError>>>,
    calls: Mutex<Vec<SessionCall>>,
    slow_call: Mutex<Option<(Duration, FakeClock)>>,
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl SessionLog {
    pub(super) fn script(&self, outcome: Result<Option<String>, SessionError>) {
        self.results.lock().unwrap().push_back(outcome);
    }

    pub(super) fn calls(&self) -> Vec<SessionCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    // a session that takes wall clock time, so a cooldown recorded after the call differs from one recorded before it
    pub(super) fn advance_on_call(&self, by: Duration, clock: FakeClock) {
        *self.slow_call.lock().unwrap() = Some((by, clock));
    }
}

#[derive(Clone)]
pub(super) struct FakeSession(Arc<SessionLog>);

impl SessionProbe for FakeSession {
    fn run_noop(
        &self,
        server: &MCPServer,
        static_headers: Option<&BTreeMap<String, String>>,
        timeout: Duration,
    ) -> impl Future<Output = Result<Option<String>, SessionError>> + Send {
        let log = Arc::clone(&self.0);
        let call = SessionCall {
            server_id: server.fields.server_id.clone(),
            timeout,
            static_headers: static_headers.is_some(),
        };
        async move {
            let running = log.active.fetch_add(1, Ordering::SeqCst).saturating_add(1);
            log.peak.fetch_max(running, Ordering::SeqCst);
            log.calls.lock().unwrap().push(call);
            tokio::task::yield_now().await;
            log.active.fetch_sub(1, Ordering::SeqCst);
            let slow = log.slow_call.lock().unwrap().clone();
            if let Some((by, clock)) = slow {
                clock.advance(by);
            }
            log.results.lock().unwrap().pop_front().unwrap_or(Ok(None))
        }
    }
}

#[derive(Default)]
pub(super) struct EnvLog {
    per_user: Mutex<BTreeSet<String>>,
    checked: Mutex<Vec<String>>,
}

impl EnvLog {
    pub(super) fn mark_per_user(&self, server_id: &str) {
        self.per_user.lock().unwrap().insert(server_id.to_owned());
    }

    pub(super) fn checked(&self) -> Vec<String> {
        self.checked.lock().unwrap().clone()
    }
}

#[derive(Clone)]
pub(super) struct FakeEnvVars(Arc<EnvLog>);

impl EnvVarCheck for FakeEnvVars {
    fn references_per_user_env_var(&self, server: &MCPServer) -> bool {
        self.0
            .checked
            .lock()
            .unwrap()
            .push(server.fields.server_id.clone());
        self.0
            .per_user
            .lock()
            .unwrap()
            .contains(&server.fields.server_id)
    }
}

#[derive(Default)]
pub(super) struct AllowedLog {
    ids: Mutex<Vec<String>>,
    calls: Mutex<Vec<bool>>,
}

impl AllowedLog {
    pub(super) fn set_ids(&self, ids: Vec<String>) {
        *self.ids.lock().unwrap() = ids;
    }

    pub(super) fn calls(&self) -> Vec<bool> {
        self.calls.lock().unwrap().clone()
    }
}

#[derive(Clone)]
pub(super) struct FakeAllowed(Arc<AllowedLog>);

impl AllowedServers for FakeAllowed {
    fn allowed_server_ids(&self, auth: Option<&UserAPIKeyAuth>) -> Vec<String> {
        self.0.calls.lock().unwrap().push(auth.is_some());
        self.0.ids.lock().unwrap().clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SpecCall {
    pub spec_path: String,
    pub max_bytes: usize,
}

#[derive(Default)]
pub(super) struct LoaderLog {
    results: Mutex<VecDeque<Result<(), OpenApiError>>>,
    calls: Mutex<Vec<SpecCall>>,
    cancel: Mutex<Option<CancellationToken>>,
    held_open: AtomicBool,
}

impl LoaderLog {
    pub(super) fn script(&self, outcome: Result<(), OpenApiError>) {
        self.results.lock().unwrap().push_back(outcome);
    }

    pub(super) fn calls(&self) -> Vec<SpecCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn hold_open(&self) {
        self.held_open.store(true, Ordering::SeqCst);
    }

    pub(super) fn cancel_while_held(&self, cancel: CancellationToken) {
        self.held_open.store(true, Ordering::SeqCst);
        *self.cancel.lock().unwrap() = Some(cancel);
    }
}

#[derive(Clone)]
pub(super) struct FakeLoader(Arc<LoaderLog>);

impl SpecLoader for FakeLoader {
    fn load(
        &self,
        spec_path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<(), OpenApiError>> + Send {
        let log = Arc::clone(&self.0);
        let call = SpecCall {
            spec_path: spec_path.to_owned(),
            max_bytes,
        };
        async move {
            log.calls.lock().unwrap().push(call);
            if let Some(cancel) = log.cancel.lock().unwrap().clone() {
                cancel.cancel();
            }
            if log.held_open.load(Ordering::SeqCst) {
                pending::<()>().await;
            }
            log.results.lock().unwrap().pop_front().unwrap_or(Ok(()))
        }
    }
}

#[derive(Clone)]
pub(super) struct FakeClock(Arc<ClockState>);

struct ClockState {
    elapsed: Mutex<Duration>,
    now: DateTime<Utc>,
}

impl FakeClock {
    pub(super) fn new() -> Self {
        Self(Arc::new(ClockState {
            elapsed: Mutex::new(Duration::ZERO),
            now: DateTime::parse_from_rfc3339(NOW_TEXT)
                .unwrap()
                .with_timezone(&Utc),
        }))
    }

    pub(super) fn advance(&self, by: Duration) {
        let mut elapsed = self.0.elapsed.lock().unwrap();
        *elapsed = elapsed.saturating_add(by);
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now_monotonic(&self) -> Duration {
        *self.0.elapsed.lock().unwrap()
    }

    fn now_utc(&self) -> DateTime<Utc> {
        let elapsed = *self.0.elapsed.lock().unwrap();
        let millis = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
        let offset = TimeDelta::try_milliseconds(millis).unwrap_or(TimeDelta::zero());
        self.0.now + offset
    }
}
