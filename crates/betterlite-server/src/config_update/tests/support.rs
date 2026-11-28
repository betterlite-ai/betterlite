#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Bytes, to_bytes};
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::settings::{JsonValue, Section};
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, FixedOffset};

use super::super::route::update_config;
use super::super::store::ConfigUpdateStore;
use super::super::{ConfigUpdateState, SettingsMap};
use crate::audit_events::callbacks::{
    AuditCallbackConfig, AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture,
    AuditCallbackRegistry, AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::AuditSettings;
use crate::audit_events::sink::{
    AuditClock, AuditEventSink, AuditLogRepository, AuditRepositoryError,
};
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::deployment_sync_port::DeploymentSyncPort;
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::state::ProxyConfigState;

pub(crate) const CONFIG_PATH: &str = "/etc/betterlite/config.yaml";

pub(crate) fn parsed(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(crate) fn settings(pairs: &[(&str, JsonValue)]) -> SettingsMap {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn object(text: &str) -> SettingsMap {
    parsed(text)
        .as_object()
        .expect("the expected JSON is an object")
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub(crate) fn config_state() -> ProxyConfigState {
    ProxyConfigState::new(PathBuf::from(CONFIG_PATH), true)
}

fn context(subject: &str, source_key: SourceKey) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

pub(crate) fn admin() -> AuthContext {
    master_caller()
}

pub(crate) fn user() -> AuthContext {
    context("user-1", SourceKey::default())
}

pub(crate) fn key_user(fields: &str) -> AuthContext {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields decode");
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key constructs");
    let mut caller = context("key-owner", SourceKey(Some(key)));
    caller.principal.roles.push(Role::PlatformAdmin);
    caller
}

#[derive(Default)]
pub(crate) struct RecordingStore {
    sections: Mutex<BTreeMap<String, SettingsMap>>,
    reads: Mutex<Vec<String>>,
    upserts: Mutex<Vec<(String, SettingsMap)>>,
    seals: Mutex<Vec<SettingsMap>>,
}

impl RecordingStore {
    pub(crate) fn holding(entries: &[(Section, SettingsMap)]) -> Arc<Self> {
        let store = Self::default();
        for (section, values) in entries {
            store
                .sections
                .lock()
                .expect("sections")
                .insert(section.name().to_owned(), values.clone());
        }
        Arc::new(store)
    }

    pub(crate) fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("reads").clone()
    }

    pub(crate) fn upserts(&self) -> Vec<(String, SettingsMap)> {
        self.upserts.lock().expect("upserts").clone()
    }

    pub(crate) fn seals(&self) -> Vec<SettingsMap> {
        self.seals.lock().expect("seals").clone()
    }

    pub(crate) fn stored(&self, section: Section) -> Option<SettingsMap> {
        self.sections
            .lock()
            .expect("sections")
            .get(section.name())
            .cloned()
    }
}

fn sealed(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::String(text) => JsonValue::String(format!("sealed:{text}")),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => value.clone(),
    }
}

#[async_trait]
impl ConfigUpdateStore for RecordingStore {
    async fn read_section(&self, section: Section) -> Result<SettingsMap, ConfigStateError> {
        self.reads
            .lock()
            .expect("reads")
            .push(section.name().to_owned());
        Ok(self.stored(section).unwrap_or_default())
    }

    async fn upsert_section(
        &self,
        section: Section,
        values: &SettingsMap,
    ) -> Result<(), ConfigStateError> {
        self.upserts
            .lock()
            .expect("upserts")
            .push((section.name().to_owned(), values.clone()));
        self.sections
            .lock()
            .expect("sections")
            .insert(section.name().to_owned(), values.clone());
        Ok(())
    }

    fn seal_environment(&self, values: &SettingsMap) -> Result<SettingsMap, ConfigStateError> {
        self.seals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(values.clone());
        Ok(values
            .iter()
            .map(|(key, value)| (key.clone(), sealed(value)))
            .collect())
    }
}

#[derive(Default)]
pub(crate) struct CollectingSpawner {
    tasks: Mutex<VecDeque<AuditCallbackFuture>>,
}

impl CollectingSpawner {
    pub(crate) async fn run_all(&self) {
        loop {
            let queued = self.tasks.lock().expect("tasks").pop_front();
            let Some(task) = queued else {
                return;
            };
            task.await;
        }
    }
}

impl AuditTaskSpawner for CollectingSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        self.tasks.lock().expect("tasks").push_back(task);
    }
}

pub(crate) struct RecordingRepository {
    pub(crate) available: bool,
    pub(crate) fail: bool,
    inserted: Mutex<Vec<AuditLogRecord>>,
}

impl RecordingRepository {
    pub(crate) fn records(&self) -> Vec<AuditLogRecord> {
        self.inserted.lock().expect("inserted").clone()
    }
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        self.available
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        if self.fail {
            return Err(AuditRepositoryError::Unavailable);
        }
        self.inserted.lock().expect("inserted").push(record.clone());
        Ok(())
    }
}

struct EnabledSettings;

impl AuditSettings for EnabledSettings {
    fn is_audit_logging_enabled(&self, _explicit: Option<bool>) -> bool {
        true
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &[]
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        None
    }
}

struct NoRegistry;

impl AuditCallbackRegistry for NoRegistry {
    fn resolve(&self, _name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        None
    }

    fn build_dedicated_s3_logger(
        &self,
        _params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        Err(AuditCallbackError::Failed)
    }
}

struct FixedClock {
    now: DateTime<FixedOffset>,
}

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

#[derive(Default)]
pub(crate) struct Deployments {
    added: AtomicUsize,
}

impl Deployments {
    pub(crate) fn count(&self) -> usize {
        self.added.load(Ordering::SeqCst)
    }
}

impl DeploymentSyncPort for Deployments {
    fn add_deployment(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let _previous = self.added.fetch_add(1, Ordering::SeqCst);
        })
    }
}

pub(crate) struct Harness {
    pub(crate) state: ConfigUpdateState,
    pub(crate) store: Arc<RecordingStore>,
    pub(crate) repository: Arc<RecordingRepository>,
    pub(crate) spawner: Arc<CollectingSpawner>,
    pub(crate) deployments: Arc<Deployments>,
}

impl Harness {
    pub(crate) fn new(config: ProxyConfigState) -> Self {
        let store = Arc::new(RecordingStore::default());
        let store_handle = Arc::clone(&store);
        let database: Arc<dyn ConfigUpdateStore> = store_handle;
        Self::with_store(store, config, Some(database), recording_repository(false))
    }

    pub(crate) fn holding(config: ProxyConfigState, store: Arc<RecordingStore>) -> Self {
        let store_handle = Arc::clone(&store);
        let database: Arc<dyn ConfigUpdateStore> = store_handle;
        Self::with_store(store, config, Some(database), recording_repository(false))
    }

    pub(crate) fn without_database(config: ProxyConfigState) -> Self {
        Self::with_store(
            Arc::new(RecordingStore::default()),
            config,
            None,
            recording_repository(false),
        )
    }

    pub(crate) fn failing_audit(config: ProxyConfigState) -> Self {
        let store = Arc::new(RecordingStore::default());
        let store_handle = Arc::clone(&store);
        let database: Arc<dyn ConfigUpdateStore> = store_handle;
        Self::with_store(store, config, Some(database), recording_repository(true))
    }

    fn with_store(
        store: Arc<RecordingStore>,
        config: ProxyConfigState,
        database: Option<Arc<dyn ConfigUpdateStore>>,
        repository: Arc<RecordingRepository>,
    ) -> Self {
        let spawner = Arc::new(CollectingSpawner::default());
        let deployments = Arc::new(Deployments::default());
        let deployments_arc = Arc::clone(&deployments);
        let deployments_handle: Arc<dyn DeploymentSyncPort> = deployments_arc;
        let audit = audit_sink(&repository, &spawner);
        let spawner_arc = Arc::clone(&spawner);
        let spawner_handle: Arc<dyn AuditTaskSpawner> = spawner_arc;
        Self {
            state: ConfigUpdateState {
                config: Arc::new(config),
                database,
                audit,
                deployments: deployments_handle,
                spawner: spawner_handle,
            },
            store,
            repository,
            spawner,
            deployments,
        }
    }
}

fn recording_repository(failing: bool) -> Arc<RecordingRepository> {
    Arc::new(RecordingRepository {
        available: true,
        fail: failing,
        inserted: Mutex::new(Vec::new()),
    })
}

fn audit_sink(
    repository: &Arc<RecordingRepository>,
    spawner: &Arc<CollectingSpawner>,
) -> Arc<AuditEventSink> {
    let spawner_arc = Arc::clone(spawner);
    let spawner_handle: Arc<dyn AuditTaskSpawner> = spawner_arc;
    let repository_arc = Arc::clone(repository);
    let repository_handle: Arc<dyn AuditLogRepository> = repository_arc;
    let clock = DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00").expect("the clock");
    Arc::new(AuditEventSink {
        settings: Arc::new(EnabledSettings),
        callbacks: Arc::new(AuditCallbackDispatcher::new(
            Arc::new(NoRegistry),
            spawner_handle,
        )),
        repository: repository_handle,
        clock: Arc::new(FixedClock { now: clock }),
        new_id: Arc::new(|| "audit-id".to_owned()),
    })
}

pub(crate) async fn send(
    harness: &Harness,
    caller: Option<AuthContext>,
    body: &str,
) -> (StatusCode, JsonValue) {
    let response = Box::pin(update_config(
        State(harness.state.clone()),
        caller.map(Extension),
        Bytes::from(body.to_owned()),
    ))
    .await;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the answer body reads");
    (
        status,
        serde_json::from_slice(&bytes).expect("the answer is JSON"),
    )
}

pub(crate) fn error_text(body: &JsonValue) -> &str {
    body.get("error")
        .and_then(|error| error.get("message"))
        .and_then(JsonValue::as_str)
        .expect("an error message")
}

pub(crate) fn error_field<'body>(body: &'body JsonValue, field: &str) -> &'body JsonValue {
    body.get("error")
        .and_then(|error| error.get(field))
        .expect("an error field")
}
