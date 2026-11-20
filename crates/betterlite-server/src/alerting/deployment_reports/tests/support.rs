#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::latency_alerts::api_base::ApiBaseResolver;
use betterlite_models::api_base_lookup::ApiBaseError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde_json::Number;
use tempfile::NamedTempFile;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::alerting::deployment_reports::DeploymentReports;
use crate::alerting::deployment_reports::daily_report::DailyReportRouter;
use crate::alerting::deployment_reports::daily_report_scheduler::ReportJitter;
use crate::alerting::deployment_reports::deployment_metrics::DailyReportCache;
use crate::alerting::model_alerts::ModelAlertSender;
use crate::alerting::model_alerts::model_deprecation::{AlertPodLock, AlertSleeper};
use crate::model_deprecation::clock::UtcClock;

pub(crate) const NOW_SECONDS: f64 = 1_767_225_600.0_f64;
pub(crate) const SENT_KEY: &str = "daily_metrics_report_sent";

#[derive(Clone)]
pub(crate) struct Journal<T>(Arc<Mutex<Vec<T>>>);

impl<T> Default for Journal<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl<T: Clone> Journal<T> {
    pub(crate) fn record(&self, entry: T) {
        self.0.lock().expect("journal lock").push(entry);
    }

    pub(crate) fn entries(&self) -> Vec<T> {
        self.0.lock().expect("journal lock").clone()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Increment {
    pub(crate) key: String,
    pub(crate) amount: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CacheWrite {
    pub(crate) key: String,
    pub(crate) value: Json,
}

#[derive(Clone)]
pub(crate) struct FakeCache {
    entries: Arc<Mutex<BTreeMap<String, Json>>>,
    pub(crate) reads: Journal<String>,
    pub(crate) writes: Journal<CacheWrite>,
    pub(crate) increments: Journal<Increment>,
    pub(crate) batch_reads: Journal<Vec<String>>,
    pub(crate) batches: Journal<Vec<(String, Json)>>,
    fails_batch: bool,
}

impl Default for FakeCache {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            reads: Journal::default(),
            writes: Journal::default(),
            increments: Journal::default(),
            batch_reads: Journal::default(),
            batches: Journal::default(),
            fails_batch: false,
        }
    }
}

impl FakeCache {
    pub(crate) fn seeded(entries: &[(&str, Json)]) -> Self {
        let cache = Self::default();
        {
            let mut stored = cache.entries.lock().expect("cache lock");
            for (key, value) in entries {
                stored.insert((*key).to_owned(), value.clone());
            }
        }
        cache
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            reads: Journal::default(),
            writes: Journal::default(),
            increments: Journal::default(),
            batch_reads: Journal::default(),
            batches: Journal::default(),
            fails_batch: true,
        }
    }

    pub(crate) fn value(&self, key: &str) -> Option<Json> {
        self.entries.lock().expect("cache lock").get(key).cloned()
    }
}

#[async_trait]
impl DailyReportCache for FakeCache {
    async fn get(&self, key: &str) -> Option<Json> {
        self.reads.record(key.to_owned());
        self.entries.lock().expect("cache lock").get(key).cloned()
    }

    async fn set(&self, key: &str, value: &Json) {
        self.entries
            .lock()
            .expect("cache lock")
            .insert(key.to_owned(), value.clone());
        self.writes.record(CacheWrite {
            key: key.to_owned(),
            value: value.clone(),
        });
    }

    async fn increment(&self, key: &str, amount: f64) {
        self.increments.record(Increment {
            key: key.to_owned(),
            amount,
        });
    }

    async fn batch_get(&self, keys: &[String]) -> Option<Vec<Option<Json>>> {
        self.batch_reads.record(keys.to_vec());
        if self.fails_batch {
            return None;
        }
        let entries = self.entries.lock().expect("cache lock");
        Some(keys.iter().map(|key| entries.get(key).cloned()).collect())
    }

    async fn set_many(&self, entries: &[(String, Json)]) {
        self.batches.record(entries.to_vec());
        let mut stored = self.entries.lock().expect("cache lock");
        for (key, value) in entries {
            stored.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SentAlert {
    pub(crate) message: String,
    pub(crate) level: AlertLevel,
    pub(crate) alert_type: AlertType,
    pub(crate) metadata: IndexMap<String, Json>,
    pub(crate) kwargs: IndexMap<String, Json>,
    pub(crate) request_model: Option<String>,
    pub(crate) api_base: Option<String>,
}

impl SentAlert {
    pub(crate) fn of(alert: Alert<'_>) -> Self {
        Self {
            message: alert.message.to_owned(),
            level: alert.level,
            alert_type: alert.alert_type,
            metadata: alert.alerting_metadata.clone(),
            kwargs: alert.kwargs.clone(),
            request_model: alert.request_model.map(str::to_owned),
            api_base: alert.api_base.map(str::to_owned),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake sender failed")]
pub(crate) struct FakeSendError;

#[derive(Clone, Default)]
pub(crate) struct FakeSender {
    pub(crate) alerts: Journal<SentAlert>,
    fails: bool,
}

impl FakeSender {
    pub(crate) fn failing() -> Self {
        Self {
            alerts: Journal::default(),
            fails: true,
        }
    }
}

#[async_trait]
impl ModelAlertSender for FakeSender {
    type Error = FakeSendError;

    async fn send_alert(&self, alert: Alert<'_>) -> Result<(), Self::Error> {
        if self.fails {
            return Err(FakeSendError);
        }
        self.alerts.record(SentAlert::of(alert));
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(crate) struct FakeApiBase {
    pub(crate) bases: BTreeMap<String, String>,
    pub(crate) calls: Journal<(String, JsonObject)>,
    failing: bool,
}

impl FakeApiBase {
    pub(crate) fn with_bases(bases: &[(&str, &str)]) -> Self {
        let mut by_model = BTreeMap::new();
        for (model, base) in bases {
            by_model.insert((*model).to_owned(), (*base).to_owned());
        }
        Self {
            bases: by_model,
            calls: Journal::default(),
            failing: false,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            bases: BTreeMap::new(),
            calls: Journal::default(),
            failing: true,
        }
    }
}

impl ApiBaseResolver for FakeApiBase {
    fn resolve(
        &self,
        model: &str,
        params: &IndexMap<String, Json>,
    ) -> Result<Option<String>, ApiBaseError> {
        self.calls.record((model.to_owned(), params.clone()));
        if self.failing {
            return Err(ApiBaseError::InvalidVertexLocation {
                message: "the fake api base failed".to_owned(),
            });
        }
        Ok(self.bases.get(model).cloned())
    }
}

#[derive(Clone, Default)]
pub(crate) struct FakeRouter {
    pub(crate) ids: Vec<String>,
    pub(crate) deployments: BTreeMap<String, DeploymentSpec>,
    pub(crate) id_calls: Journal<()>,
    pub(crate) info_calls: Journal<String>,
}

impl DailyReportRouter for FakeRouter {
    fn model_ids(&self) -> Vec<String> {
        self.id_calls.record(());
        self.ids.clone()
    }

    fn model_info(&self, id: &str) -> Option<DeploymentSpec> {
        self.info_calls.record(id.to_owned());
        self.deployments.get(id).cloned()
    }
}

#[derive(Clone)]
pub(crate) struct FakeSleeper {
    pub(crate) sleeps: Journal<u64>,
    answers: Arc<Mutex<VecDeque<bool>>>,
}

impl FakeSleeper {
    pub(crate) fn with_answers(answers: Vec<bool>) -> Self {
        Self {
            sleeps: Journal::default(),
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
        }
    }
}

#[async_trait]
impl AlertSleeper for FakeSleeper {
    async fn sleep(&self, seconds: u64) -> bool {
        self.sleeps.record(seconds);
        self.answers
            .lock()
            .expect("sleeper lock")
            .pop_front()
            .unwrap_or(false)
    }
}

#[derive(Clone)]
pub(crate) struct FakeJitter {
    pub(crate) bounds: Journal<(i64, i64)>,
    answers: Arc<Mutex<VecDeque<i64>>>,
}

impl FakeJitter {
    pub(crate) fn with_answers(answers: Vec<i64>) -> Self {
        Self {
            bounds: Journal::default(),
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
        }
    }
}

impl ReportJitter for FakeJitter {
    fn between(&self, low: i64, high: i64) -> i64 {
        self.bounds.record((low, high));
        self.answers
            .lock()
            .expect("jitter lock")
            .pop_front()
            .unwrap_or(low)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LockCall {
    pub(crate) cronjob_id: String,
    pub(crate) ttl: Option<i64>,
    pub(crate) allow_reentrant: bool,
}

#[derive(Clone)]
pub(crate) struct FakePodLock {
    answer: Option<bool>,
    pub(crate) calls: Journal<LockCall>,
}

impl FakePodLock {
    pub(crate) fn returning(answer: Option<bool>) -> Self {
        Self {
            answer,
            calls: Journal::default(),
        }
    }
}

#[async_trait]
impl AlertPodLock for FakePodLock {
    async fn acquire_lock(
        &self,
        cronjob_id: &str,
        ttl: Option<i64>,
        allow_reentrant: bool,
    ) -> Option<bool> {
        self.calls.record(LockCall {
            cronjob_id: cronjob_id.to_owned(),
            ttl,
            allow_reentrant,
        });
        self.answer
    }
}

#[derive(Clone)]
pub(crate) struct FixedClock(pub(crate) DateTime<Utc>);

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) type TestReports =
    DeploymentReports<FakeCache, FakeSender, FakeApiBase, FixedClock, FakeSleeper, FakeJitter>;

pub(crate) fn fixed_now() -> DateTime<Utc> {
    "2026-01-01T00:00:00Z".parse().expect("a fixed instant")
}

pub(crate) fn reports() -> TestReports {
    DeploymentReports {
        alert_types: vec![AlertType::DailyReports],
        alerting_args: SlackAlertingArgs::default(),
        cache: FakeCache::default(),
        sender: FakeSender::default(),
        api_base: FakeApiBase::default(),
        clock: FixedClock(fixed_now()),
        sleeper: FakeSleeper::with_answers(vec![false]),
        jitter: FakeJitter::with_answers(vec![300]),
    }
}

pub(crate) fn int_metric(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(crate) fn float_metric(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite metric"))
}

pub(crate) fn deployment(id: &str, model: &str, api_base: Option<&str>) -> DeploymentSpec {
    let mut model_info = IndexMap::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    let mut litellm_params = IndexMap::new();
    litellm_params.insert("model".to_owned(), Json::Text(model.to_owned()));
    if let Some(base) = api_base {
        litellm_params.insert("api_base".to_owned(), Json::Text(base.to_owned()));
    }
    DeploymentSpec {
        model_name: "group".to_owned(),
        litellm_params,
        model_info,
        extra: IndexMap::new(),
    }
}

pub(crate) struct LogCapture {
    _guard: DefaultGuard,
    file: NamedTempFile,
}

impl LogCapture {
    pub(crate) fn new() -> Self {
        let file = NamedTempFile::new().expect("a temporary log file");
        let writer = file.reopen().expect("a second handle on the log file");
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(Level::ERROR)
            .with_writer(Arc::new(writer))
            .finish();
        Self {
            _guard: set_default(subscriber),
            file,
        }
    }

    pub(crate) fn text(&self) -> String {
        fs::read_to_string(self.file.path()).expect("the captured log")
    }
}

pub(crate) fn router_with(deployments: Vec<DeploymentSpec>) -> FakeRouter {
    let mut ids = Vec::new();
    let mut by_id = BTreeMap::new();
    for spec in deployments {
        let Some(id) = spec.id().map(str::to_owned) else {
            continue;
        };
        ids.push(id.clone());
        by_id.insert(id, spec);
    }
    FakeRouter {
        ids,
        deployments: by_id,
        id_calls: Journal::default(),
        info_calls: Journal::default(),
    }
}
