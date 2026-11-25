#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_db::health_check_rows::{HealthCheckRow, HealthCheckRowsError, NewHealthCheck};
use betterlite_db::ptu::store::sleeper::Sleeper;
use betterlite_db::queues::pod_lock::owner::{PodId, lease_key};
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, Utc};
use serde_json::Number;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::auth::auth_identity::AuthContext;
use crate::health_cycle::endpoints::HealthCheckResult;
use crate::health_cycle::ports::ProbeFailure;
use crate::health_persistence::HealthCheckStore;
use crate::health_shared::background::BackgroundHealthLoop;
use crate::health_shared::cache::SharedHealthCache;
use crate::health_shared::error::{SharedHealthCacheError, SharedHealthError};
use crate::health_shared::health_state::HealthStateWriter;
use crate::health_shared::manager::SharedHealthCheckManager;
use crate::health_shared::probe::{SharedHealthProbe, SharedProbeCall};
use crate::health_shared::results::HealthResultsCache;
use crate::health_shared::settings::{DEFAULT_SETTINGS, SharedHealthSettings};
use crate::health_shared::status_view::{SharedHealthStatusError, SharedHealthStatusSource};
use crate::model_deprecation::clock::UtcClock;

pub(crate) const POD_A: &str = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
pub(crate) const POD_B: &str = "9c858901-8a57-4791-81fe-4c455b099bc9";
pub(crate) const SHARED_AT: &str = "2024-01-01T00:00:00Z";
pub(crate) const SHARED_SECONDS: f64 = 1_704_067_200.0;
pub(crate) const CHECKER: &str = "pod_1704067200000";

pub(crate) fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("a fixture instant")
        .with_timezone(&Utc)
}

pub(crate) fn number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite fixture number"))
}

pub(crate) struct FixedClock(pub(crate) DateTime<Utc>);

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) fn healthy_row(id: &str) -> JsonObject {
    let mut row = JsonObject::new();
    row.insert("model".to_owned(), Json::Text(format!("openai/{id}")));
    row.insert("model_id".to_owned(), Json::Text(id.to_owned()));
    row
}

pub(crate) fn unhealthy_row(id: &str, status: u16) -> JsonObject {
    let mut row = healthy_row(id);
    row.insert("error".to_owned(), Json::Text(format!("{id} is down")));
    row.insert(
        "exception_status".to_owned(),
        Json::Number(Number::from(u64::from(status))),
    );
    row
}

pub(crate) fn probe_failure(id: &str, status: u16) -> ProbeFailure {
    ProbeFailure {
        message: format!("{id} is down"),
        status_code: status,
        raw_request_typed_dict: None,
    }
}

pub(crate) fn probe_result() -> HealthCheckResult {
    let mut exceptions = BTreeMap::new();
    exceptions.insert("probe-down".to_owned(), probe_failure("probe-down", 503));
    HealthCheckResult {
        healthy_endpoints: vec![healthy_row("probe-ok")],
        unhealthy_endpoints: vec![unhealthy_row("probe-down", 503)],
        exceptions_by_model_id: exceptions,
    }
}

pub(crate) fn empty_call() -> SharedProbeCall<'static> {
    SharedProbeCall {
        models: &[],
        details: true,
        max_concurrency: None,
        skip_disabled: true,
        instrumentation: None,
    }
}

pub(crate) fn field(object: &JsonObject, name: &str) -> Json {
    object
        .get(name)
        .expect("the fixture object carries the field")
        .clone()
}

pub(crate) fn keys(object: &JsonObject) -> Vec<String> {
    object.keys().cloned().collect()
}

pub(crate) fn written_payload(world: &World) -> JsonObject {
    let call = world
        .cache
        .sets()
        .into_iter()
        .find(|call| call.key == SharedHealthCheckManager::CACHE_KEY)
        .expect("the results were cached");
    serde_json::from_str(&call.value).expect("the cached payload is JSON")
}

pub(crate) fn cached_text(timestamp: f64, checked_by: &str) -> String {
    let mut payload = JsonObject::new();
    payload.insert(
        "healthy_endpoints".to_owned(),
        Json::List(vec![Json::Object(healthy_row("cached-ok"))]),
    );
    payload.insert(
        "unhealthy_endpoints".to_owned(),
        Json::List(vec![Json::Object(unhealthy_row("cached-down", 500))]),
    );
    payload.insert(
        "healthy_count".to_owned(),
        Json::Number(Number::from(1_u32)),
    );
    payload.insert(
        "unhealthy_count".to_owned(),
        Json::Number(Number::from(1_u32)),
    );
    payload.insert("timestamp".to_owned(), number(timestamp));
    payload.insert("checked_by".to_owned(), Json::Text(checked_by.to_owned()));
    serde_json::to_string(&payload).expect("the fixture payload serializes")
}

pub(crate) fn deployment(name: &str, id: &str, disabled: bool) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(format!("openai/{name}")));
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(id.to_owned()));
    if disabled {
        info.insert(
            "disable_background_health_check".to_owned(),
            Json::Bool(true),
        );
    }
    DeploymentSpec::new(
        name.to_owned(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .expect("the fixture deployment is valid")
}

pub(crate) fn caller() -> AuthContext {
    let principal: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"user","roles":[],"auth_method":"api_key"}"#,
    )
    .expect("the fixture principal");
    AuthContext { principal }
}

pub(crate) async fn route_get(
    router: Router,
    caller: Option<&AuthContext>,
    uri: &str,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("the fixture request builds");
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    let response = router.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the fixture response body");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("the response is UTF-8"),
    )
}

#[derive(Default)]
pub(crate) struct Ops(Mutex<Vec<String>>);

impl Ops {
    pub(crate) fn record(&self, operation: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(operation.to_owned());
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    Get,
    Set,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetCall {
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) ttl: i64,
    pub(crate) nx: bool,
}

#[derive(Default)]
struct FakeState {
    values: BTreeMap<String, String>,
    sets: Vec<SetCall>,
    deletes: Vec<String>,
    hidden: BTreeMap<String, usize>,
    faults: BTreeMap<&'static str, String>,
}

#[derive(Default)]
pub(crate) struct FakeCache {
    ops: Arc<Ops>,
    state: Mutex<FakeState>,
}

impl FakeCache {
    pub(crate) fn new(ops: Arc<Ops>) -> Arc<Self> {
        Arc::new(Self {
            ops,
            state: Mutex::new(FakeState::default()),
        })
    }

    pub(crate) fn put_plain(&self, key: &str, value: &str) {
        let stored = serde_json::to_string(value).expect("a fixture value serializes");
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values
            .insert(key.to_owned(), stored);
    }

    pub(crate) fn put_json(&self, key: &str, text: &str) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values
            .insert(key.to_owned(), text.to_owned());
    }

    pub(crate) fn hide_for(&self, key: &str, reads: usize) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .hidden
            .insert(key.to_owned(), reads);
    }

    pub(crate) fn fail(&self, fault: Fault, key: &str) {
        let name = match fault {
            Fault::Get => "get",
            Fault::Set => "set",
            Fault::Delete => "delete",
        };
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .faults
            .insert(name, key.to_owned());
    }

    pub(crate) fn sets(&self) -> Vec<SetCall> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sets
            .clone()
    }

    pub(crate) fn deletes(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .deletes
            .clone()
    }

    pub(crate) fn has(&self, key: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values
            .contains_key(key)
    }

    pub(crate) fn value(&self, key: &str) -> Option<CachedValue> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let raw = state.values.get(key)?;
        serde_json::from_str(raw).ok()
    }
}

fn cache_failure(operation: &'static str) -> SharedHealthCacheError {
    SharedHealthCacheError {
        operation,
        source: Box::new(FixtureFailure),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fixture cache failed")]
struct FixtureFailure;

#[async_trait]
impl SharedHealthCache for FakeCache {
    async fn get(&self, key: &str) -> Result<Option<CachedValue>, SharedHealthCacheError> {
        self.ops.record(&format!("get:{key}"));
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.faults.get("get").is_some_and(|failed| failed == key) {
            return Err(cache_failure("async_get_cache"));
        }
        let hidden = match state.hidden.get_mut(key) {
            Some(remaining) => {
                let hit = *remaining > 0;
                *remaining = remaining.saturating_sub(1);
                hit
            }
            None => false,
        };
        if hidden {
            return Ok(None);
        }
        let raw = match state.values.get(key) {
            Some(raw) => raw.clone(),
            None => return Ok(None),
        };
        Ok(serde_json::from_str(&raw).ok())
    }

    async fn set(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: i64,
        only_if_absent: bool,
    ) -> Result<bool, SharedHealthCacheError> {
        self.ops.record(&format!("set:{key}"));
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.sets.push(SetCall {
            key: key.to_owned(),
            value: value.to_owned(),
            ttl: ttl_seconds,
            nx: only_if_absent,
        });
        if state.faults.get("set").is_some_and(|failed| failed == key) {
            return Err(cache_failure("async_set_cache"));
        }
        if only_if_absent && state.values.contains_key(key) {
            return Ok(false);
        }
        let stored = serde_json::to_string(value).unwrap_or_default();
        state.values.insert(key.to_owned(), stored);
        Ok(true)
    }

    async fn delete(&self, key: &str) -> Result<i64, SharedHealthCacheError> {
        self.ops.record(&format!("delete:{key}"));
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state
            .faults
            .get("delete")
            .is_some_and(|failed| failed == key)
        {
            return Err(cache_failure("async_delete_cache"));
        }
        state.deletes.push(key.to_owned());
        Ok(match state.values.remove(key) {
            Some(_stored) => 1,
            None => 0,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProbeCall {
    pub(crate) model_count: usize,
    pub(crate) details: bool,
    pub(crate) max_concurrency: Option<i64>,
    pub(crate) skip_disabled: bool,
    pub(crate) cycle_id: Option<String>,
}

pub(crate) struct FakeProbe {
    ops: Arc<Ops>,
    calls: Mutex<Vec<ProbeCall>>,
    outcomes: Mutex<VecDeque<Result<HealthCheckResult, SharedHealthError>>>,
    reply: Mutex<HealthCheckResult>,
}

impl FakeProbe {
    pub(crate) fn new(ops: Arc<Ops>, reply: HealthCheckResult) -> Arc<Self> {
        Arc::new(Self {
            ops,
            calls: Mutex::new(Vec::new()),
            outcomes: Mutex::new(VecDeque::new()),
            reply: Mutex::new(reply),
        })
    }

    pub(crate) fn calls(&self) -> Vec<ProbeCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn queue(&self, outcome: Result<HealthCheckResult, SharedHealthError>) {
        self.outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(outcome);
    }
}

#[async_trait]
impl SharedHealthProbe for FakeProbe {
    async fn probe(
        &self,
        call: SharedProbeCall<'_>,
    ) -> Result<HealthCheckResult, SharedHealthError> {
        self.ops.record("probe");
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ProbeCall {
                model_count: call.models.len(),
                details: call.details,
                max_concurrency: call.max_concurrency,
                skip_disabled: call.skip_disabled,
                cycle_id: call.instrumentation.map(|context| context.cycle_id.clone()),
            });
        let queued = self
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match queued {
            Some(outcome) => outcome,
            None => Ok(self
                .reply
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SaveOutcome {
    #[default]
    Ok,
    Fail,
    Hang,
}

#[derive(Default)]
struct StoreState {
    reads: usize,
    rows: Vec<NewHealthCheck>,
    outcome: SaveOutcome,
}

#[derive(Default)]
pub(crate) struct FakeStore {
    state: Mutex<StoreState>,
    entered: Notify,
    never: Notify,
}

impl FakeStore {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn reads(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reads
    }

    pub(crate) fn rows(&self) -> Vec<NewHealthCheck> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .clone()
    }

    pub(crate) fn behave(&self, outcome: SaveOutcome) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .outcome = outcome;
    }

    pub(crate) async fn saved_once(&self) {
        self.entered.notified().await;
    }
}

fn stored_row(new: &NewHealthCheck) -> HealthCheckRow {
    let at = instant(SHARED_AT);
    HealthCheckRow {
        health_check_id: "saved".to_owned(),
        model_name: new.model_name.clone(),
        model_id: new.model_id.clone(),
        status: new.status.clone(),
        healthy_count: new.healthy_count,
        unhealthy_count: new.unhealthy_count,
        error_message: new.error_message.clone(),
        response_time_ms: new.response_time_ms,
        details: None,
        checked_by: new.checked_by.clone(),
        checked_at: at,
        created_at: at,
        updated_at: at,
    }
}

impl HealthCheckStore for FakeStore {
    async fn stored_checks(&self) -> Result<Vec<HealthCheckRow>, HealthCheckRowsError> {
        let outcome = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.reads = state.reads.saturating_add(1);
            state.outcome
        };
        if outcome == SaveOutcome::Hang {
            self.entered.notify_one();
            self.never.notified().await;
        }
        Ok(Vec::new())
    }

    async fn persist_result(&self, new: &NewHealthCheck) -> Option<HealthCheckRow> {
        let outcome = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.rows.push(new.clone());
            state.outcome
        };
        self.entered.notify_one();
        match outcome {
            SaveOutcome::Ok => Some(stored_row(new)),
            SaveOutcome::Fail => None,
            SaveOutcome::Hang => {
                self.never.notified().await;
                None
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WriteCall {
    pub(crate) healthy: Vec<JsonObject>,
    pub(crate) unhealthy: Vec<JsonObject>,
    pub(crate) exceptions: BTreeMap<String, ProbeFailure>,
}

#[derive(Default)]
pub(crate) struct FakeWriter {
    writes: Mutex<Vec<WriteCall>>,
}

impl FakeWriter {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn writes(&self) -> Vec<WriteCall> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl HealthStateWriter for FakeWriter {
    fn write(
        &self,
        healthy: &[JsonObject],
        unhealthy: &[JsonObject],
        exceptions: &BTreeMap<String, ProbeFailure>,
    ) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(WriteCall {
                healthy: healthy.to_vec(),
                unhealthy: unhealthy.to_vec(),
                exceptions: exceptions.clone(),
            });
    }
}

#[derive(Default)]
pub(crate) struct FakeStatus {
    calls: AtomicUsize,
    body: JsonObject,
    failure: Option<String>,
}

impl FakeStatus {
    pub(crate) fn answering(body: JsonObject) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            body,
            failure: None,
        })
    }

    pub(crate) fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            body: JsonObject::new(),
            failure: Some(message.to_owned()),
        })
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl SharedHealthStatusSource for FakeStatus {
    async fn status(&self) -> Result<JsonObject, SharedHealthStatusError> {
        let _previous = self.calls.fetch_add(1, Ordering::Relaxed);
        match &self.failure {
            Some(message) => Err(SharedHealthStatusError {
                message: message.clone(),
            }),
            None => Ok(self.body.clone()),
        }
    }
}

#[derive(Default)]
pub(crate) struct FakeLock {
    values: Mutex<BTreeMap<String, String>>,
    deletes: Mutex<Vec<String>>,
}

impl FakeLock {
    pub(crate) fn holds(&self, job: &str) -> bool {
        let key = lease_key(job);
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&key)
    }

    pub(crate) fn released(&self, job: &str) -> bool {
        let key = lease_key(job);
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|deleted| deleted == &key)
    }
}

impl LockCache for FakeLock {
    async fn get_owner(&self, key: &str) -> Result<Option<Vec<u8>>, LockCacheError> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(values.get(key).map(|owner| owner.as_bytes().to_vec()))
    }

    async fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        _ttl_seconds: i64,
    ) -> Result<bool, LockCacheError> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.contains_key(key) {
            return Ok(false);
        }
        values.insert(key.to_owned(), owner.to_owned());
        Ok(true)
    }

    async fn delete(&self, key: &str) -> Result<i64, LockCacheError> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        Ok(match values.remove(key) {
            Some(_owner) => 1,
            None => 0,
        })
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingSleeper {
    delays: Arc<Mutex<Vec<Duration>>>,
}

impl RecordingSleeper {
    pub(crate) fn delays(&self) -> Vec<Duration> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Sleeper for RecordingSleeper {
    async fn sleep(&mut self, duration: Duration) {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
    }
}

#[derive(Clone, Default)]
pub(crate) struct StepSleeper {
    pub(crate) slept: Arc<Notify>,
    pub(crate) wakes: Arc<Notify>,
    delays: Arc<Mutex<Vec<Duration>>>,
}

impl StepSleeper {
    pub(crate) fn delays(&self) -> Vec<Duration> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Sleeper for StepSleeper {
    async fn sleep(&mut self, duration: Duration) {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        self.slept.notify_one();
        self.wakes.notified().await;
    }
}

pub(crate) async fn run_until_sleep<F>(sleeper: &StepSleeper, future: &mut Pin<Box<F>>)
where
    F: Future<Output = ()>,
{
    tokio::select! {
        () = future => panic!("the background loop returned"),
        () = sleeper.slept.notified() => {}
    }
}

pub(crate) struct World {
    now: DateTime<Utc>,
    pub(crate) ops: Arc<Ops>,
    pub(crate) cache: Arc<FakeCache>,
    pub(crate) probe: Arc<FakeProbe>,
    pub(crate) store: Arc<FakeStore>,
    pub(crate) writer: Arc<FakeWriter>,
    pub(crate) lock: Arc<FakeLock>,
    pub(crate) results: Arc<HealthResultsCache>,
    pub(crate) tasks: TaskTracker,
}

impl World {
    pub(crate) fn new(now: DateTime<Utc>) -> Self {
        let ops = Arc::new(Ops::default());
        Self {
            now,
            cache: FakeCache::new(Arc::clone(&ops)),
            probe: FakeProbe::new(Arc::clone(&ops), probe_result()),
            store: FakeStore::new(),
            writer: FakeWriter::new(),
            lock: Arc::new(FakeLock::default()),
            results: Arc::new(HealthResultsCache::default()),
            tasks: TaskTracker::new(),
            ops,
        }
    }

    pub(crate) fn clock(&self) -> Arc<dyn UtcClock + Send + Sync> {
        Arc::new(FixedClock(self.now))
    }

    pub(crate) fn manager(&self) -> Arc<SharedHealthCheckManager> {
        self.manager_with(DEFAULT_SETTINGS)
    }

    pub(crate) fn manager_with(
        &self,
        settings: SharedHealthSettings,
    ) -> Arc<SharedHealthCheckManager> {
        Arc::new(SharedHealthCheckManager::new(
            Some(Arc::clone(&self.cache)),
            settings,
            self.clock(),
        ))
    }

    pub(crate) fn local_manager(&self) -> Arc<SharedHealthCheckManager> {
        Arc::new(SharedHealthCheckManager::new(
            None,
            DEFAULT_SETTINGS,
            self.clock(),
        ))
    }

    pub(crate) fn pod_manager(&self, pod: &str) -> Arc<PodLockManager<FakeLock>> {
        let pod_id =
            PodId::try_from(Uuid::parse_str(pod).expect("a fixture uuid")).expect("a v4 pod id");
        Arc::new(PodLockManager::new(
            Some(Arc::clone(&self.lock)),
            60,
            Arc::new(NoPodLockEventSink),
            pod_id,
        ))
    }

    pub(crate) fn bare_pod_manager(&self) -> Arc<PodLockManager<FakeLock>> {
        let pod_id =
            PodId::try_from(Uuid::parse_str(POD_A).expect("a fixture uuid")).expect("a v4 pod id");
        Arc::new(PodLockManager::new(
            None,
            60,
            Arc::new(NoPodLockEventSink),
            pod_id,
        ))
    }

    pub(crate) fn background(
        &self,
        manager: Option<Arc<SharedHealthCheckManager>>,
        deployments: Vec<DeploymentSpec>,
    ) -> BackgroundHealthLoop<FakeStore, FakeLock> {
        BackgroundHealthLoop {
            interval_seconds: 300,
            details: true,
            max_concurrency: None,
            manager,
            probe: Arc::clone(&self.probe),
            deployments: Some(Arc::new(deployments)),
            results: Arc::clone(&self.results),
            store: Some(Arc::clone(&self.store)),
            lock: Some(self.pod_manager(POD_A)),
            health_state: Some(Arc::clone(&self.writer)),
            clock: self.clock(),
            tasks: self.tasks.clone(),
        }
    }
}
