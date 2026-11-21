use std::collections::VecDeque;
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::health_check_rows::{
    HealthCheckRow, HealthCheckRows, HealthCheckRowsError, NewHealthCheck,
};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, TimeDelta, Utc};
use tokio::task::yield_now;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::health_persistence::writes::latest_by_key;
use crate::health_persistence::{AggregatedHealthResult, HealthCheckStore, LatestChecks};
use crate::model_deprecation::clock::UtcClock;

pub(super) const BROKEN_LATEST: &str = "the details column holds text that is not JSON";

pub(super) fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

pub(super) fn ago(now: DateTime<Utc>, seconds: i64) -> DateTime<Utc> {
    now.checked_sub_signed(TimeDelta::seconds(seconds)).unwrap()
}

pub(super) fn deployment_json(model_name: &str, param: Json, id: Option<Json>) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), param);
    let mut model_info = JsonObject::new();
    if let Some(model_info_id) = id {
        model_info.insert("id".to_owned(), model_info_id);
    }
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params,
        model_info,
        extra: JsonObject::new(),
    }
}

pub(super) fn deployment(model_name: &str, param: &str, id: Option<&str>) -> DeploymentSpec {
    let model_info_id = id.map(|value| Json::Text(value.to_owned()));
    deployment_json(model_name, Json::Text(param.to_owned()), model_info_id)
}

pub(super) fn endpoint(model: &str, error: Option<&str>) -> JsonObject {
    let mut result = JsonObject::new();
    result.insert("model".to_owned(), Json::Text(model.to_owned()));
    if let Some(message) = error {
        result.insert("error".to_owned(), Json::Text(message.to_owned()));
    }
    result
}

pub(super) fn result_of(
    model_name: &str,
    model_id: Option<&str>,
    counts: (i32, i32),
) -> AggregatedHealthResult {
    AggregatedHealthResult {
        model_name: model_name.to_owned(),
        model_id: model_id.map(str::to_owned),
        healthy_count: counts.0,
        unhealthy_count: counts.1,
        error_message: None,
    }
}

pub(super) fn latest_row(
    model_name: &str,
    model_id: Option<&str>,
    status: &str,
    checked_at: DateTime<Utc>,
) -> HealthCheckRow {
    HealthCheckRow {
        health_check_id: format!("id-{model_name}"),
        model_name: model_name.to_owned(),
        model_id: model_id.map(str::to_owned),
        status: status.to_owned(),
        healthy_count: 0,
        unhealthy_count: 0,
        error_message: None,
        response_time_ms: None,
        details: None,
        checked_by: None,
        checked_at,
        created_at: checked_at,
        updated_at: checked_at,
    }
}

pub(super) fn latest_map(rows: Vec<HealthCheckRow>) -> LatestChecks {
    latest_by_key(rows)
}

// the real store with no pool: every call reaches the driver's "No DB Connected" error without a database
pub(super) fn unconnected() -> HealthCheckRows {
    HealthCheckRows {
        store: PostgresStore { pool: None },
        new_id: Arc::new(|| String::from("unused-id")),
    }
}

fn stored_row(new: &NewHealthCheck) -> HealthCheckRow {
    HealthCheckRow {
        health_check_id: String::from("saved"),
        model_name: new.model_name.clone(),
        model_id: new.model_id.clone(),
        status: new.status.clone(),
        healthy_count: new.healthy_count,
        unhealthy_count: new.unhealthy_count,
        error_message: new.error_message.clone(),
        response_time_ms: new.response_time_ms,
        details: None,
        checked_by: new.checked_by.clone(),
        checked_at: at(0),
        created_at: at(0),
        updated_at: at(0),
    }
}

fn broken_error() -> HealthCheckRowsError {
    let source = serde_json::from_str::<Json>("{").unwrap_err();
    HealthCheckRowsError::Details { source }
}

pub(super) struct FixedClock(pub(super) DateTime<Utc>);

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(super) struct ScriptedClock {
    readings: Mutex<VecDeque<DateTime<Utc>>>,
    fallback: DateTime<Utc>,
    calls: AtomicUsize,
}

impl ScriptedClock {
    pub(super) fn new(readings: Vec<DateTime<Utc>>) -> Self {
        Self {
            readings: Mutex::new(readings.into_iter().collect()),
            fallback: at(0),
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl UtcClock for ScriptedClock {
    fn now(&self) -> DateTime<Utc> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut readings = self.readings.lock().unwrap_or_else(PoisonError::into_inner);
        readings.pop_front().unwrap_or(self.fallback)
    }
}

pub(super) struct FakeStore {
    latest: Vec<HealthCheckRow>,
    latest_fails: bool,
    fail_for: Vec<String>,
    saved: Mutex<Vec<NewHealthCheck>>,
    latest_calls: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

impl FakeStore {
    pub(super) fn new(latest: Vec<HealthCheckRow>) -> Self {
        Self {
            latest,
            latest_fails: false,
            fail_for: Vec::new(),
            saved: Mutex::new(Vec::new()),
            latest_calls: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        }
    }

    pub(super) fn with_broken_latest() -> Self {
        let mut store = Self::new(Vec::new());
        store.latest_fails = true;
        store
    }

    pub(super) fn failing_for(mut self, names: &[&str]) -> Self {
        self.fail_for = names.iter().copied().map(String::from).collect();
        self
    }

    pub(super) fn saved(&self) -> Vec<NewHealthCheck> {
        self.saved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn latest_calls(&self) -> usize {
        self.latest_calls.load(Ordering::SeqCst)
    }

    pub(super) fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    pub(super) fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }

    fn latest_rows(&self) -> Result<Vec<HealthCheckRow>, HealthCheckRowsError> {
        self.latest_calls.fetch_add(1, Ordering::SeqCst);
        if self.latest_fails {
            return Err(broken_error());
        }
        Ok(self.latest.clone())
    }

    async fn record_save(&self, new: &NewHealthCheck) -> Option<HealthCheckRow> {
        let current = self
            .in_flight
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.max_in_flight.fetch_max(current, Ordering::SeqCst);
        let failed = self.fail_for.contains(&new.model_name);
        let stored = stored_row(new);
        self.saved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(new.clone());
        yield_now().await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        if failed { None } else { Some(stored) }
    }
}

impl HealthCheckStore for FakeStore {
    fn stored_checks(
        &self,
    ) -> impl Future<Output = Result<Vec<HealthCheckRow>, HealthCheckRowsError>> + Send {
        ready(self.latest_rows())
    }

    fn persist_result(
        &self,
        new: &NewHealthCheck,
    ) -> impl Future<Output = Option<HealthCheckRow>> + Send {
        self.record_save(new)
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(super) fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl io::Write for Logs {
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

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (Logs, DefaultGuard) {
    let buffer = Logs::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
