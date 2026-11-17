use std::collections::{BTreeMap, BTreeSet};
use std::future::pending;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::cooldown_state::cooldown::{ActiveCooldown, CooldownCache};
use crate::cooldown_state::health::{DeploymentHealthCache, HealthSnapshot, HealthState};
use crate::cooldown_state::record::CooldownRecord;
use crate::cooldown_state::settings::Settings;
use crate::cooldown_state::shared::{HealthReadFuture, SharedCooldowns, SharedHealth};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_read_batch::store::{BatchRead, BatchReadFuture, ReadWarning};
use crate::routing_read_batch::{RequestReads, RoutingReadBatch};
use crate::routing_value_store::{Clock, RouteKey};

pub(crate) struct TestClock(Arc<Mutex<f64>>);

impl TestClock {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(0.0_f64)))
    }

    pub(crate) fn set(&self, seconds: f64) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = seconds;
    }

    pub(crate) fn handle(&self) -> Clock {
        let source = Arc::clone(&self.0);
        Arc::new(move || *source.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Written {
    pub(crate) key: RouteKey,
    pub(crate) record: CooldownRecord,
    pub(crate) ttl_seconds: f64,
}

#[derive(Default)]
pub(crate) struct MemoryCooldowns {
    records: Mutex<BTreeMap<RouteKey, CooldownRecord>>,
    writes: Mutex<Vec<Written>>,
    reads: Mutex<Vec<Vec<RouteKey>>>,
    warning: Option<ReadWarning>,
    hanging: AtomicBool,
}

impl MemoryCooldowns {
    pub(crate) fn with(records: &[(&str, CooldownRecord)]) -> Self {
        let mut stored = BTreeMap::new();
        for (model_id, record) in records {
            stored.insert(key(model_id), record.clone());
        }
        Self {
            records: Mutex::new(stored),
            writes: Mutex::new(Vec::new()),
            reads: Mutex::new(Vec::new()),
            warning: None,
            hanging: AtomicBool::new(false),
        }
    }

    pub(crate) fn warning(mut self, warning: ReadWarning) -> Self {
        self.warning = Some(warning);
        self
    }

    pub(crate) fn hanging(self) -> Self {
        self.hanging.store(true, Ordering::Relaxed);
        self
    }

    pub(crate) fn stop_hanging(&self) {
        self.hanging.store(false, Ordering::Relaxed);
    }

    pub(crate) fn set(&self, model_id: &str, record: CooldownRecord) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key(model_id), record);
    }

    pub(crate) fn writes(&self) -> Vec<Written> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn reads(&self) -> Vec<Vec<RouteKey>> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SharedCooldowns for MemoryCooldowns {
    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, CooldownRecord> {
        Box::pin(async move {
            self.reads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(keys.to_vec());
            if self.hanging.load(Ordering::Relaxed) {
                return pending::<BatchRead<CooldownRecord>>().await;
            }
            let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
            BatchRead {
                values: keys.iter().map(|key| records.get(key).cloned()).collect(),
                warning: self.warning,
            }
        })
    }

    fn write(&self, key: &RouteKey, record: &CooldownRecord, ttl_seconds: f64) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Written {
                key: key.clone(),
                record: record.clone(),
                ttl_seconds,
            });
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), record.clone());
    }
}

#[derive(Default)]
pub(crate) struct MemoryHealth {
    raw: Mutex<Option<HealthSnapshot>>,
    writes: Mutex<Vec<(HealthSnapshot, i64)>>,
    reads: AtomicUsize,
}

impl MemoryHealth {
    pub(crate) fn with(snapshot: HealthSnapshot) -> Self {
        Self {
            raw: Mutex::new(Some(snapshot)),
            writes: Mutex::new(Vec::new()),
            reads: AtomicUsize::new(0),
        }
    }

    pub(crate) fn clear_raw(&self) {
        *self.raw.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    pub(crate) fn set_raw(&self, snapshot: HealthSnapshot) {
        *self.raw.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
    }

    pub(crate) fn raw(&self) -> Option<HealthSnapshot> {
        self.raw
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn writes(&self) -> Vec<(HealthSnapshot, i64)> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn read_count(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }
}

impl SharedHealth for MemoryHealth {
    fn read<'store>(&'store self) -> HealthReadFuture<'store> {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.raw
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    }

    fn read_raw(&self) -> Option<HealthSnapshot> {
        self.raw()
    }

    fn write(&self, snapshot: &HealthSnapshot, ttl_seconds: i64) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((snapshot.clone(), ttl_seconds));
        *self.raw.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot.clone());
    }
}

pub(crate) async fn read_at(
    cache: &CooldownCache,
    clock: &TestClock,
    at: f64,
    model_ids: &[&str],
) -> Vec<Option<CooldownRecord>> {
    clock.set(at);
    let keys = ids(model_ids);
    let mut batch = RoutingReadBatch::default();
    let mut request = RequestReads::default();
    cache.read(&mut batch, &mut request, &keys).await.unwrap()
}

pub(crate) async fn active_at(
    cache: &CooldownCache,
    clock: &TestClock,
    at: f64,
    model_id: &str,
) -> Vec<ActiveCooldown> {
    let keys = ids(&[model_id]);
    let results = read_at(cache, clock, at, &[model_id]).await;
    cache.active_from_results(&keys, &results).unwrap()
}

pub(crate) fn key(model_id: &str) -> RouteKey {
    RouteKey::parse(&format!("deployment:{model_id}:cooldown")).unwrap()
}

pub(crate) fn ids(model_ids: &[&str]) -> Vec<String> {
    model_ids
        .iter()
        .map(|model_id| (*model_id).to_owned())
        .collect()
}

pub(crate) fn record(timestamp: f64, cooldown_time: f64) -> CooldownRecord {
    CooldownRecord {
        exception_received: String::from("boom"),
        status_code: String::from("429"),
        timestamp,
        cooldown_time,
    }
}

pub(crate) fn unhealthy(timestamp: f64) -> HealthState {
    HealthState {
        is_healthy: false,
        timestamp,
        reason: String::from("probe failed"),
    }
}

pub(crate) fn healthy(timestamp: f64) -> HealthState {
    HealthState {
        is_healthy: true,
        timestamp,
        reason: String::new(),
    }
}

pub(crate) fn snapshot(states: &[(&str, HealthState)]) -> HealthSnapshot {
    states
        .iter()
        .map(|(model_id, state)| ((*model_id).to_owned(), state.clone()))
        .collect()
}

pub(crate) fn settings() -> Settings {
    Settings {
        default_cooldown_time: 5.0,
        redis_read_interval_seconds: 1.0,
        enable_health_check_routing: true,
        staleness_threshold: 10.0,
        ignore_transient_errors: false,
    }
}

pub(crate) fn cache(
    clock: &TestClock,
    shared: Arc<MemoryCooldowns>,
    configured: &Settings,
) -> CooldownCache {
    CooldownCache::new(configured, shared, clock.handle())
}

pub(crate) fn health(
    clock: &TestClock,
    shared: Arc<MemoryHealth>,
    staleness_threshold: f64,
) -> DeploymentHealthCache {
    DeploymentHealthCache::new(staleness_threshold, shared, clock.handle())
}

pub(crate) fn deployment(model_name: &str, id: &str) -> DeploymentSpec {
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(
        model_name.to_owned(),
        JsonObject::new(),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

pub(crate) fn flagged(mut deployment: DeploymentSpec, field: &str, value: Json) -> DeploymentSpec {
    deployment.model_info.insert(field.to_owned(), value);
    deployment
}

pub(crate) fn pool(entries: &[(&str, &str)]) -> Vec<DeploymentSpec> {
    entries
        .iter()
        .map(|(model_name, id)| deployment(model_name, id))
        .collect()
}

pub(crate) fn kept(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .filter_map(DeploymentSpec::id)
        .map(str::to_owned)
        .collect()
}

pub(crate) fn unhealthy_ids(model_ids: &[&str]) -> BTreeSet<String> {
    model_ids
        .iter()
        .map(|model_id| (*model_id).to_owned())
        .collect()
}
