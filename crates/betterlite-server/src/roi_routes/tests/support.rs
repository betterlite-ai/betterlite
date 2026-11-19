#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_repository::config::ConfigError;
use betterlite_roi::records::{ROIReport, ROISyncPhase, ROISyncStatus};
use betterlite_roi::settings::ROISettings;
use betterlite_secrets::error::SecretError;
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::auth::auth_identity::AuthContext;
use crate::roi_connections::error::RoiConnectionsError;
use crate::roi_connections::gateway_key::MISSING_KEY;
use crate::roi_estimator::CompletionCaller;
use crate::roi_github::error::SourceError;
use crate::roi_routes::error::RoiRoutesError;
use crate::roi_routes::ports::{
    CompletionFactory, RouteSyncRun, SpendFuture, SpendReaderFactory, SpendReads, SyncManagerPort,
    SyncStorePort, SyncStoreSource,
};
use crate::roi_routes::state::RoiRouteState;
use crate::roi_settings::deployments::{RouterCatalog, RouterEstimatorDeployment};
use crate::roi_settings::repository::SettingsRepository;
use crate::roi_settings::store::{REPORT_KEY, SETTINGS_KEY, SettingsCipher};

pub(crate) const SEALED_TOKEN: &str = "cipher-github-token";
pub(crate) const SEALED_ESTIMATOR: &str = "cipher-estimator-key";
pub(crate) const FIXED_OWNER: u128 = 7;

pub(crate) type Calls = Arc<Mutex<Vec<String>>>;

pub(crate) fn recorded(calls: &Calls) -> Vec<String> {
    calls.lock().expect("calls lock").clone()
}

pub(crate) fn caller(subject: &str, roles: &[Role]) -> AuthContext {
    let roles_json = serde_json::to_string(roles).expect("roles json");
    let raw = format!(
        r#"{{"principal_type":"human","subject":"{subject}","roles":{roles_json},"auth_method":"api_key"}}"#
    );
    let principal: Principal = serde_json::from_str(&raw).expect("test principal");
    AuthContext { principal }
}

pub(crate) fn admin() -> AuthContext {
    caller("user", &[Role::PlatformAdmin])
}

pub(crate) fn viewer() -> AuthContext {
    caller("user", &[Role::PlatformViewer])
}

pub(crate) fn member() -> AuthContext {
    caller("user", &[Role::OrgAdmin])
}

pub(crate) struct MemoryRepository {
    params: Mutex<BTreeMap<String, JsonData>>,
    calls: Calls,
    fail_reads: AtomicBool,
    fail_writes: AtomicBool,
}

impl MemoryRepository {
    fn new(calls: Calls) -> Self {
        Self {
            params: Mutex::new(BTreeMap::new()),
            calls,
            fail_reads: AtomicBool::new(false),
            fail_writes: AtomicBool::new(false),
        }
    }

    pub(crate) fn seed(&self, name: &str, raw: &str) {
        let value: JsonData = serde_json::from_str(raw).expect("seeded parameter");
        self.params
            .lock()
            .expect("repository lock")
            .insert(name.to_owned(), value);
    }

    pub(crate) fn raw(&self, name: &str) -> Option<String> {
        let value = self
            .params
            .lock()
            .expect("repository lock")
            .get(name)
            .cloned()?;
        Some(serde_json::to_string(&value).expect("stored json"))
    }

    pub(crate) fn fail_reads(&self) {
        self.fail_reads.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_writes(&self) {
        self.fail_writes.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl SettingsRepository for MemoryRepository {
    async fn param(&self, name: &str) -> Result<Option<JsonData>, ConfigError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(ConfigError::Table(TableError::MissingRow));
        }
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("repo.param:{name}"));
        let params = self.params.lock().expect("repository lock");
        Ok(params.get(name).cloned())
    }

    async fn set_param(&self, name: &str, value: JsonData) -> Result<(), ConfigError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("repo.set:{name}"));
        let failed = self.fail_writes.load(Ordering::SeqCst);
        if failed {
            return Err(ConfigError::Table(TableError::MissingRow));
        }
        self.params
            .lock()
            .expect("repository lock")
            .insert(name.to_owned(), value);
        Ok(())
    }
}

pub(crate) struct PlainCipher;

impl SettingsCipher for PlainCipher {
    fn seal(&self, value: &SecretString) -> Result<String, SecretError> {
        Ok(value.expose_secret().to_owned())
    }

    fn open(&self, value: &str) -> SecretString {
        SecretString::from(value.to_owned())
    }
}

#[derive(Default)]
pub(crate) struct Catalog {
    pub(crate) names: Vec<String>,
    pub(crate) deployments: Vec<RouterEstimatorDeployment>,
}

impl RouterCatalog for Catalog {
    fn model_names(&self) -> Vec<String> {
        self.names.clone()
    }

    fn deployments(&self, _model_group: &str) -> Vec<RouterEstimatorDeployment> {
        self.deployments.clone()
    }
}

pub(crate) fn deployment(model: &str) -> RouterEstimatorDeployment {
    serde_json::from_str(&format!(
        r#"{{"litellm_params":{{"model":"{model}","custom_llm_provider":"openai"}}}}"#
    ))
    .expect("deployment")
}

pub(crate) struct NoSpend;

impl SpendReads for NoSpend {
    fn read_spend(&self, _start: chrono::NaiveDate, _end: chrono::NaiveDate) -> SpendFuture<'_> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

pub(crate) fn completion_factory() -> CompletionFactory {
    Arc::new(|_settings| {
        let caller: CompletionCaller = Arc::new(|_request| {
            Box::pin(async { Err::<_, SourceError>(SourceError::PaginationLimit) })
        });
        Ok(caller)
    })
}

pub(crate) struct FakeStore {
    calls: Calls,
    status: Mutex<Option<ROISyncStatus>>,
    acquire_result: AtomicBool,
    fail_status: AtomicBool,
    fail_clear: AtomicBool,
    fail_finish: AtomicBool,
    refuse_heartbeat: AtomicBool,
    acquire_owner: Mutex<Option<String>>,
    acquire_status: Mutex<Option<ROISyncStatus>>,
    acquire_interval: Mutex<Option<f64>>,
    finished: Mutex<Option<ROISyncStatus>>,
    cleared: AtomicBool,
}

impl FakeStore {
    pub(crate) fn set_status(&self, status: ROISyncStatus) {
        *self.status.lock().expect("store status lock") = Some(status);
    }

    pub(crate) fn refuse_acquire(&self) {
        self.acquire_result.store(false, Ordering::SeqCst);
    }

    pub(crate) fn fail_status(&self) {
        self.fail_status.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_clear(&self) {
        self.fail_clear.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_finish(&self) {
        self.fail_finish.store(true, Ordering::SeqCst);
    }

    pub(crate) fn refuse_heartbeat(&self) {
        self.refuse_heartbeat.store(true, Ordering::SeqCst);
    }

    pub(crate) fn acquired_owner(&self) -> Option<String> {
        self.acquire_owner.lock().expect("owner lock").clone()
    }

    pub(crate) fn acquired_status(&self) -> Option<ROISyncStatus> {
        self.acquire_status.lock().expect("acquire lock").clone()
    }

    pub(crate) fn acquired_interval(&self) -> Option<f64> {
        *self.acquire_interval.lock().expect("interval lock")
    }

    pub(crate) fn finished_status(&self) -> Option<ROISyncStatus> {
        self.finished.lock().expect("finished lock").clone()
    }

    pub(crate) fn cleared(&self) -> bool {
        self.cleared.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SyncStorePort for FakeStore {
    async fn read_status(&self) -> Result<Option<ROISyncStatus>, RoiRoutesError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push("store.status".to_owned());
        if self.fail_status.load(Ordering::SeqCst) {
            return Err(RoiRoutesError::Store(DbError::NoDbConnected));
        }
        Ok(self.status.lock().expect("store status lock").clone())
    }

    async fn cancel_sync(&self) -> Result<(), RoiRoutesError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push("store.cancel".to_owned());
        Ok(())
    }

    async fn acquire_lease(
        &self,
        owner: &str,
        status: &ROISyncStatus,
        scheduled_interval: f64,
    ) -> Result<bool, RoiRoutesError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push("store.acquire".to_owned());
        *self.acquire_owner.lock().expect("owner lock") = Some(owner.to_owned());
        *self.acquire_status.lock().expect("acquire lock") = Some(status.clone());
        *self.acquire_interval.lock().expect("interval lock") = Some(scheduled_interval);
        Ok(self.acquire_result.load(Ordering::SeqCst))
    }

    async fn heartbeat_lease(
        &self,
        _owner: &str,
        _status: &ROISyncStatus,
    ) -> Result<bool, RoiRoutesError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push("store.heartbeat".to_owned());
        Ok(!self.refuse_heartbeat.load(Ordering::SeqCst))
    }

    async fn finish_claim(
        &self,
        _owner: &str,
        status: &ROISyncStatus,
        _report: Option<&ROIReport>,
    ) -> Result<bool, RoiRoutesError> {
        let call = "store.finish".to_owned();
        self.calls.lock().expect("calls lock").push(call);
        *self.finished.lock().expect("finished lock") = Some(status.clone());
        if self.fail_finish.load(Ordering::SeqCst) {
            return Err(RoiRoutesError::Store(DbError::NoDbConnected));
        }
        Ok(true)
    }

    async fn clear_stored_report(&self) -> Result<(), RoiRoutesError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push("store.clear_report".to_owned());
        self.cleared.store(true, Ordering::SeqCst);
        if self.fail_clear.load(Ordering::SeqCst) {
            return Err(RoiRoutesError::Store(DbError::NoDbConnected));
        }
        Ok(())
    }
}

#[async_trait]
impl SyncStoreSource for Arc<FakeStore> {
    async fn store(&self) -> Result<Arc<dyn SyncStorePort>, RoiRoutesError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push("store.open".to_owned());
        let store: Arc<dyn SyncStorePort> = Self::clone(self);
        Ok(store)
    }
}

#[derive(Clone)]
pub(crate) struct RecordedRun {
    pub(crate) model: String,
    pub(crate) interval: f64,
    pub(crate) coordinated: bool,
    pub(crate) repos: Vec<String>,
}

pub(crate) struct FakeManager {
    calls: Calls,
    status: Mutex<ROISyncStatus>,
    start_result: AtomicBool,
    status_after_start: Mutex<Option<ROISyncStatus>>,
    runs: Mutex<Vec<RecordedRun>>,
}

impl FakeManager {
    pub(crate) fn set_status(&self, status: ROISyncStatus) {
        *self.status.lock().expect("manager status lock") = status;
    }

    pub(crate) fn accept_start(&self, status: ROISyncStatus) {
        self.start_result.store(true, Ordering::SeqCst);
        *self.status_after_start.lock().expect("status lock") = Some(status);
    }

    pub(crate) fn runs(&self) -> Vec<RecordedRun> {
        let runs = self.runs.lock().expect("runs lock");
        runs.clone()
    }
}

#[async_trait]
impl SyncManagerPort for FakeManager {
    fn current_status(&self) -> ROISyncStatus {
        self.calls
            .lock()
            .expect("calls lock")
            .push("manager.status".to_owned());
        self.status.lock().expect("manager status lock").clone()
    }

    async fn start_run(&self, run: RouteSyncRun) -> bool {
        self.calls
            .lock()
            .expect("calls lock")
            .push("manager.start".to_owned());
        self.runs.lock().expect("runs lock").push(RecordedRun {
            model: run.settings.estimator_model.clone(),
            interval: run.scheduled_interval,
            coordinated: run.store.is_some(),
            repos: run.settings.repos.clone(),
        });
        let accepted = self.start_result.load(Ordering::SeqCst);
        if accepted {
            let next = self.status_after_start.lock().expect("status lock").clone();
            if let Some(status) = next {
                *self.status.lock().expect("manager status lock") = status;
            }
        }
        accepted
    }

    async fn cancel_active_run(&self) {
        self.calls
            .lock()
            .expect("calls lock")
            .push("manager.cancel".to_owned());
    }
}

pub(crate) fn idle_status() -> ROISyncStatus {
    ROISyncStatus {
        running: false,
        phase: ROISyncPhase::Idle,
        stage: "Idle".to_owned(),
        done: 0,
        total: 0,
        estimated: 0,
        reused: 0,
        needs_attention: 0,
        error: None,
        started_at: None,
        finished_at: None,
        next_update: None,
        elapsed_seconds: 0,
        remaining_seconds: None,
    }
}

pub(crate) fn finished_status(finished_at: &str) -> ROISyncStatus {
    ROISyncStatus {
        finished_at: Some(finished_at.to_owned()),
        ..idle_status()
    }
}

pub(crate) fn running_status() -> ROISyncStatus {
    let mut status = idle_status();
    status.running = true;
    status.phase = ROISyncPhase::Spend;
    status.stage = "Reading gateway spend".to_owned();
    status
}

pub(crate) fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-02-01T12:00:00+00:00")
        .expect("fixed now")
        .with_timezone(&Utc)
}

#[derive(Serialize)]
struct StoredSeed<'seed> {
    github_api_url: &'seed str,
    github_token: &'seed str,
    estimator_key: &'seed str,
    repos: &'seed [&'seed str],
    estimator_model: &'seed str,
    estimator_prompt: &'seed str,
    backfill_days: i64,
    update_interval_minutes: f64,
    identity_map: BTreeMap<&'seed str, &'seed str>,
}

pub(crate) fn stored_json(
    repos: &[&str],
    model: &str,
    interval: f64,
    identity: &[(&str, &str)],
) -> String {
    let seed = StoredSeed {
        github_api_url: "https://api.github.com",
        github_token: SEALED_TOKEN,
        estimator_key: SEALED_ESTIMATOR,
        repos,
        estimator_model: model,
        estimator_prompt: "Estimate the effort.",
        backfill_days: 7,
        update_interval_minutes: interval,
        identity_map: identity.iter().copied().collect(),
    };
    serde_json::to_string(&seed).expect("stored seed json")
}

pub(crate) fn ready_stored() -> String {
    stored_json(&["acme/widgets"], "gpt-4o", 60.0, &[])
}

pub(crate) fn settings_from(raw: &str) -> ROISettings {
    serde_json::from_str(raw).expect("settings json")
}

pub(crate) fn report_json(report: &ROIReport) -> String {
    serde_json::to_string(report).expect("report json")
}

pub(crate) fn minutes_before(base: DateTime<Utc>, minutes: i64) -> DateTime<Utc> {
    base - TimeDelta::minutes(minutes)
}

#[derive(Default, Deserialize)]
pub(crate) struct StoredPeek {
    #[serde(default)]
    pub(crate) github_token: String,
    #[serde(default)]
    pub(crate) estimator_key: String,
    #[serde(default)]
    pub(crate) repos: Vec<String>,
    #[serde(default)]
    pub(crate) estimator_model: String,
    #[serde(default)]
    pub(crate) identity_map: BTreeMap<String, String>,
}

pub(crate) fn stored_peek(harness: &Harness) -> StoredPeek {
    let raw = harness
        .repository
        .raw(SETTINGS_KEY)
        .expect("stored settings");
    serde_json::from_str(&raw).expect("stored settings json")
}

fn catalog_with(names: &[&str]) -> Catalog {
    Catalog {
        names: names.iter().copied().map(str::to_owned).collect(),
        deployments: names.iter().copied().map(deployment).collect(),
    }
}

pub(crate) struct Harness {
    pub(crate) calls: Calls,
    pub(crate) repository: Arc<MemoryRepository>,
    pub(crate) catalog: Arc<Catalog>,
    pub(crate) store: Arc<FakeStore>,
    pub(crate) manager: Arc<FakeManager>,
    pub(crate) repository_present: bool,
    pub(crate) now: DateTime<Utc>,
    pub(crate) completion: CompletionFactory,
}

impl Harness {
    pub(crate) fn new() -> Self {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let store = FakeStore {
            calls: Arc::clone(&calls),
            status: Mutex::new(None),
            acquire_result: AtomicBool::new(true),
            fail_status: AtomicBool::new(false),
            fail_clear: AtomicBool::new(false),
            fail_finish: AtomicBool::new(false),
            refuse_heartbeat: AtomicBool::new(false),
            acquire_owner: Mutex::new(None),
            acquire_status: Mutex::new(None),
            acquire_interval: Mutex::new(None),
            finished: Mutex::new(None),
            cleared: AtomicBool::new(false),
        };
        let manager = FakeManager {
            calls: Arc::clone(&calls),
            status: Mutex::new(idle_status()),
            start_result: AtomicBool::new(false),
            status_after_start: Mutex::new(None),
            runs: Mutex::new(Vec::new()),
        };
        Self {
            repository: Arc::new(MemoryRepository::new(Arc::clone(&calls))),
            catalog: Arc::new(catalog_with(&["gpt-4o"])),
            store: Arc::new(store),
            manager: Arc::new(manager),
            calls,
            repository_present: true,
            now: fixed_now(),
            completion: completion_factory(),
        }
    }

    pub(crate) fn seed_settings(&self, raw: &str) {
        self.repository.seed(SETTINGS_KEY, raw);
    }

    pub(crate) fn seed_report(&self, raw: &str) {
        self.repository.seed(REPORT_KEY, raw);
    }

    pub(crate) fn models(&mut self, names: &[&str]) {
        self.catalog = Arc::new(catalog_with(names));
    }

    pub(crate) fn without_repository(mut self) -> Self {
        self.repository_present = false;
        self
    }

    pub(crate) fn at(mut self, now: DateTime<Utc>) -> Self {
        self.now = now;
        self
    }

    pub(crate) fn with_completion(mut self, factory: CompletionFactory) -> Self {
        self.completion = factory;
        self
    }

    pub(crate) fn state(&self) -> RoiRouteState {
        let repository: Arc<dyn SettingsRepository> =
            Arc::<MemoryRepository>::clone(&self.repository);
        let catalog: Arc<dyn RouterCatalog> = Arc::<Catalog>::clone(&self.catalog);
        let store: Arc<dyn SyncStoreSource> = Arc::new(Arc::<FakeStore>::clone(&self.store));
        let manager: Arc<dyn SyncManagerPort> = Arc::<FakeManager>::clone(&self.manager);
        let now = self.now;
        let spend_reader: SpendReaderFactory =
            Arc::new(|| -> Arc<dyn SpendReads> { Arc::new(NoSpend) });
        RoiRouteState {
            repository: self.repository_present.then_some(repository),
            cipher: Arc::new(PlainCipher),
            catalog: Some(catalog),
            sync_store: store,
            manager,
            clock: Arc::new(move || now),
            spend_reader,
            completion_caller: Arc::clone(&self.completion),
            next_id: Arc::new(|| Uuid::from_u128(FIXED_OWNER)),
        }
    }

    pub(crate) fn app(&self) -> Router {
        crate::roi_routes::routes(self.state())
    }
}

pub(crate) fn request(
    method: &str,
    uri: &str,
    caller: Option<&AuthContext>,
    body: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let built = builder
        .body(body.map_or_else(Body::empty, |text| Body::from(text.to_owned())))
        .expect("request");
    with_caller(built, caller)
}

pub(crate) fn get(uri: &str, caller: Option<&AuthContext>) -> Request<Body> {
    request("GET", uri, caller, None)
}

pub(crate) fn post(uri: &str, caller: Option<&AuthContext>) -> Request<Body> {
    request("POST", uri, caller, None)
}

pub(crate) fn delete(uri: &str, caller: Option<&AuthContext>) -> Request<Body> {
    request("DELETE", uri, caller, None)
}

pub(crate) fn put(uri: &str, caller: Option<&AuthContext>, body: &str) -> Request<Body> {
    request("PUT", uri, caller, Some(body))
}

fn with_caller(mut request: Request<Body>, caller: Option<&AuthContext>) -> Request<Body> {
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    request
}

pub(crate) async fn call(app: Router, request: Request<Body>) -> Reply {
    let response = app.oneshot(request).await.expect("router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    Reply {
        status,
        body: bytes.to_vec(),
    }
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).expect("json body")
    }

    pub(crate) fn detail(&self) -> String {
        self.json::<DetailText>().detail
    }

    pub(crate) fn issues(&self) -> Vec<TestIssue> {
        self.json::<DetailIssues>().detail
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("utf-8 body")
    }
}

#[derive(Deserialize)]
struct DetailText {
    detail: String,
}

#[derive(Deserialize)]
struct DetailIssues {
    detail: Vec<TestIssue>,
}

#[derive(Deserialize)]
pub(crate) struct TestIssue {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) loc: Vec<String>,
    pub(crate) msg: String,
}

#[derive(Deserialize)]
pub(crate) struct SummaryPeek {
    pub(crate) mode: String,
    pub(crate) synced_at: String,
    pub(crate) repos: Vec<String>,
    pub(crate) people: Vec<PersonPeek>,
}

#[derive(Deserialize)]
pub(crate) struct PersonPeek {
    pub(crate) email: String,
}

#[derive(Deserialize)]
pub(crate) struct ReportReply {
    pub(crate) report: Option<SummaryPeek>,
}

#[derive(Deserialize)]
pub(crate) struct IdentityReply {
    pub(crate) report: Option<SummaryPeek>,
    pub(crate) identity_map: BTreeMap<String, String>,
}

pub(crate) fn missing_key_completion() -> CompletionFactory {
    Arc::new(|_settings| Err(RoiConnectionsError::Conflict(MISSING_KEY)))
}
