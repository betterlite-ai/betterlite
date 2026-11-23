use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_config::sections::settings::{Section, SettingsStore};
use betterlite_core::api::anthropic_beta_config::matrix::{BetaMatrix, BetaNode};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use betterlite_db::reload_schedule::ReloadSchedule;
use betterlite_db::table_ops::rows::ConfigParamRow;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::fetch_policy::outcome::Unavailable;
use betterlite_models::reload::ReloadOutcome;
use betterlite_models::source::{CatalogProvenance, CatalogSnapshot, SourceKind};
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use serde_json::json;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::periodic_reloads::PeriodicReloadChecks;
use crate::periodic_reloads::config_rows::ReloadConfigRows;
use crate::periodic_reloads::sources::{
    BetaHeadersSource, BetaMatrixFuture, CostMapFuture, ModelCostMapSource, ReloadSources,
};
use crate::periodic_reloads::state::PeriodicReloads;

const CATALOG_WITH_ALIASES: &[u8] =
    br#"{"model":{"litellm_provider":"openai","aliases":["alias"]}}"#;

pub(super) fn moment(hour: u32) -> DateTime<Utc> {
    moment_at(hour, 0)
}

pub(super) fn moment_at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 6, hour, minute, 0)
        .expect("the fixed moment is valid")
}

pub(super) fn provenance(loaded_at: Option<DateTime<Utc>>) -> CatalogProvenance {
    CatalogProvenance {
        kind: SourceKind::Local,
        url: None,
        is_env_forced: false,
        fallback_reason: None,
        loaded_at,
        source_revision: None,
        etag: None,
    }
}

pub(super) fn snapshot() -> Arc<CatalogSnapshot> {
    let catalog = ModelCatalog::parse(CATALOG_WITH_ALIASES, UnknownFields::Reject)
        .expect("the snapshot catalog parses");
    Arc::new(CatalogSnapshot {
        catalog: Arc::new(catalog),
        provenance: provenance(None),
    })
}

pub(super) fn reloaded() -> ReloadOutcome {
    ReloadOutcome::Reloaded(snapshot())
}

pub(super) fn unavailable(reason: &str) -> ReloadOutcome {
    ReloadOutcome::Unavailable(Unavailable {
        reason: reason.to_owned(),
    })
}

pub(super) fn schedule(
    revision: u32,
    interval_hours: Option<u32>,
    last_run_at: Option<DateTime<Utc>>,
) -> ReloadSchedule {
    ReloadSchedule {
        interval_hours,
        reload_revision: revision,
        last_run_at,
    }
}

// three providers beside the two keys LiteLLM's count skips, so a wrong skip shows a different number
pub(super) fn beta_matrix() -> BetaMatrix {
    BetaMatrix(IndexMap::from([
        ("anthropic".to_owned(), BetaNode::Map(IndexMap::new())),
        ("bedrock".to_owned(), BetaNode::Map(IndexMap::new())),
        ("vertex_ai".to_owned(), BetaNode::Map(IndexMap::new())),
        (
            "provider_aliases".to_owned(),
            BetaNode::Map(IndexMap::new()),
        ),
        (
            "description".to_owned(),
            BetaNode::Text("providers".to_owned()),
        ),
    ]))
}

pub(super) fn raw_beta_row(value: JsonData) -> ConfigParamRow {
    ConfigParamRow {
        param_name: "anthropic_beta_headers_reload_config".to_owned(),
        param_value: value,
    }
}

pub(super) fn beta_row(interval_hours: Option<u32>, force_reload: bool) -> ConfigParamRow {
    let interval = interval_hours.map_or(JsonData::Null, |hours| {
        JsonData::Number(serde_json::Number::from(hours))
    });
    let value = JsonData::Object(BTreeMap::from([
        ("interval_hours".to_owned(), interval),
        ("force_reload".to_owned(), JsonData::Bool(force_reload)),
    ]));
    raw_beta_row(value)
}

pub(super) fn general_settings(objects: &[&str]) -> SettingsStore {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(
        "supported_db_objects".to_owned(),
        json!(objects),
    )]));
    store
}

pub(super) fn everything_settings() -> SettingsStore {
    SettingsStore::from(Section::GeneralSettings)
}

pub(super) async fn run_cost_map_check(harness: &Harness, settings: &SettingsStore) {
    harness
        .checks
        .check_periodic_reloads(&harness.rows, settings)
        .await;
}

pub(super) async fn run_beta_check(harness: &Harness) {
    harness
        .checks
        .check_beta_headers_reload(&harness.rows)
        .await;
}

#[derive(Clone)]
pub(super) struct FakeClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl FakeClock {
    pub(super) fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    pub(super) fn set(&self, now: DateTime<Utc>) {
        let mut guard = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *guard = now;
    }
}

impl GatewayRequestClock for FakeClock {
    fn utc_now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeCostMap {
    queue: Arc<Mutex<VecDeque<ReloadOutcome>>>,
    calls: Arc<AtomicUsize>,
}

impl FakeCostMap {
    pub(super) fn queue(&self, outcome: ReloadOutcome) {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.push_back(outcome);
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ModelCostMapSource for FakeCostMap {
    fn refetch_cost_map<'call>(&'call self) -> CostMapFuture<'call> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ReloadOutcome::Unavailable(Unavailable {
                reason: "the fake cost map source has no reply queued".to_owned(),
            }));
        Box::pin(async move { outcome })
    }
}

#[derive(Clone)]
pub(super) struct FakeBetaHeaders {
    matrix: Arc<BetaMatrix>,
    calls: Arc<AtomicUsize>,
}

impl FakeBetaHeaders {
    pub(super) fn new(matrix: BetaMatrix) -> Self {
        Self {
            matrix: Arc::new(matrix),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl BetaHeadersSource for FakeBetaHeaders {
    fn reload<'call>(&'call self) -> BetaMatrixFuture<'call> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let matrix = Arc::clone(&self.matrix);
        Box::pin(async move { matrix })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake config repository refused the {operation} call")]
pub(super) struct FakeRowsError {
    operation: &'static str,
}

#[derive(Default)]
struct FakeRowsState {
    schedule: Option<ReloadSchedule>,
    beta_row: Option<ConfigParamRow>,
    failures: Vec<&'static str>,
    recorded: Vec<DateTime<Utc>>,
    writes: Vec<JsonData>,
    evictions: AtomicUsize,
}

#[derive(Clone, Default)]
pub(super) struct FakeRows {
    state: Arc<Mutex<FakeRowsState>>,
}

impl FakeRows {
    pub(super) fn set_schedule(&self, schedule: ReloadSchedule) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.schedule = Some(schedule);
    }

    pub(super) fn set_beta_row(&self, row: Option<ConfigParamRow>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.beta_row = row;
    }

    pub(super) fn fail(&self, operation: &'static str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.failures.push(operation);
    }

    pub(super) fn recorded(&self) -> Vec<DateTime<Utc>> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.recorded.clone()
    }

    pub(super) fn writes(&self) -> Vec<JsonData> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.writes.clone()
    }

    pub(super) fn evictions(&self) -> usize {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.evictions.load(Ordering::SeqCst)
    }

    fn decide(
        &self,
        operation: &'static str,
    ) -> Result<MutexGuard<'_, FakeRowsState>, FakeRowsError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.failures.contains(&operation) {
            return Err(FakeRowsError { operation });
        }
        Ok(state)
    }
}

impl ReloadConfigRows for FakeRows {
    type Error = FakeRowsError;

    async fn read_schedule(&self) -> Result<Option<ReloadSchedule>, FakeRowsError> {
        let state = self.decide("read_schedule")?;
        Ok(state.schedule.clone())
    }

    async fn record_run(&self, ran_at: DateTime<Utc>) -> Result<(), FakeRowsError> {
        let mut state = self.decide("record_run")?;
        state.recorded.push(ran_at);
        Ok(())
    }

    async fn read_beta_headers(&self) -> Result<Option<ConfigParamRow>, FakeRowsError> {
        let state = self.decide("read_beta_headers")?;
        Ok(state.beta_row.clone())
    }

    async fn write_beta_headers(&self, value: JsonData) -> Result<(), FakeRowsError> {
        let mut state = self.decide("write_beta_headers")?;
        state.writes.push(value);
        Ok(())
    }

    async fn evict_beta_headers(&self) -> Result<(), FakeRowsError> {
        let state = self.decide("evict_beta_headers")?;
        state.evictions.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(super) struct Harness {
    pub(super) rows: FakeRows,
    pub(super) clock: FakeClock,
    pub(super) cost_map: FakeCostMap,
    pub(super) beta: FakeBetaHeaders,
    pub(super) checks: PeriodicReloadChecks,
}

impl Harness {
    pub(super) fn new(now: DateTime<Utc>) -> Self {
        let clock = FakeClock::new(now);
        let cost_map = FakeCostMap::default();
        let beta = FakeBetaHeaders::new(beta_matrix());
        let checks = PeriodicReloadChecks {
            reloads: Arc::new(PeriodicReloads::new(&provenance(Some(now)), now)),
            sources: ReloadSources {
                clock: Arc::new(clock.clone()),
                cost_map: Arc::new(cost_map.clone()),
                beta_headers: Arc::new(beta.clone()),
            },
        };
        Self {
            rows: FakeRows::default(),
            clock,
            cost_map,
            beta,
            checks,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the captured log lines are UTF-8")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::TRACE)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
