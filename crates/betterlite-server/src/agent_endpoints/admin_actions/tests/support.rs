use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::{
    AgentKillSwitchConfig, AgentKillSwitchMethod, AgentKillSwitchResult,
};
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tower::ServiceExt as _;

use crate::agent_endpoints::admin_actions::ports::{
    AgentRegistryLookup, AgentRowLookup, KillSwitchAuditWriter, KillSwitchTrigger, KnownAgent,
    PortFailure, PublicGroupConfigWriter, PublicGroupStore,
};
use crate::agent_endpoints::admin_actions::{AgentAdminState, agent_admin_routes};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::AuditClock;
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;

pub(super) const MAKE_AGENTS_PUBLIC_PATH: &str = "/v1/agents/make_public";
pub(super) const ADMIN: &str = "proxy_admin";

pub(super) type Trace = Arc<Mutex<Vec<String>>>;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn new_trace() -> Trace {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn events(trace: &Trace) -> Vec<String> {
    lock(trace).clone()
}

fn log_step(trace: &Trace, event: &str) {
    lock(trace).push(event.to_owned());
}

pub(super) fn kill_switch_path(agent_id: &str) -> String {
    format!("/v1/agents/{agent_id}/kill_switch")
}

pub(super) fn make_public_path(agent_id: &str) -> String {
    format!("/v1/agents/{agent_id}/make_public")
}

pub(super) fn known(agent_id: &str, agent_name: &str) -> KnownAgent {
    KnownAgent {
        agent_id: String::from(agent_id),
        agent_name: String::from(agent_name),
        kill_switch: None,
    }
}

pub(super) fn known_with_switch(agent_id: &str, agent_name: &str, url: &str) -> KnownAgent {
    KnownAgent {
        agent_id: String::from(agent_id),
        agent_name: String::from(agent_name),
        kill_switch: Some(switch_json(url)),
    }
}

pub(super) fn switch_json(url: &str) -> AgentJson {
    serde_json::from_str(&format!(r#"{{"url":"{url}"}}"#)).unwrap()
}

pub(super) fn succeeded_result() -> AgentKillSwitchResult {
    AgentKillSwitchResult {
        agent_id: String::from("agent-1"),
        url: String::from("https://agent.example/stop"),
        method: AgentKillSwitchMethod::Post,
        status_code: Some(200),
        response_body: Some(String::from("stopped")),
        error: None,
    }
}

pub(super) fn failed_result() -> AgentKillSwitchResult {
    AgentKillSwitchResult {
        status_code: Some(500),
        response_body: Some(String::from("boom")),
        ..succeeded_result()
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct FakeFailure {
    pub(super) message: String,
}

#[derive(Clone)]
pub(super) struct FakeRegistry {
    agents: Arc<Mutex<BTreeMap<String, KnownAgent>>>,
    aliases: Arc<Mutex<BTreeMap<String, BTreeSet<String>>>>,
    lookups: Trace,
}

impl FakeRegistry {
    fn new(lookups: &Trace) -> Self {
        Self {
            agents: Arc::new(Mutex::new(BTreeMap::new())),
            aliases: Arc::new(Mutex::new(BTreeMap::new())),
            lookups: Arc::clone(lookups),
        }
    }

    pub(super) fn register(&self, agent: KnownAgent) {
        lock(&self.agents).insert(agent.agent_id.clone(), agent);
    }

    pub(super) fn alias(&self, stable: &str, legacy: &str) {
        lock(&self.aliases)
            .entry(stable.to_owned())
            .or_default()
            .insert(legacy.to_owned());
    }

    pub(super) fn resolves(&self, requested: &str, agent: KnownAgent) {
        lock(&self.agents).insert(requested.to_owned(), agent);
    }
}

impl AgentRegistryLookup for FakeRegistry {
    fn agent_by_id(&self, agent_id: &str) -> Option<KnownAgent> {
        log_step(&self.lookups, "registry");
        lock(&self.agents).get(agent_id).cloned()
    }

    fn ids_for_agent(&self, agent_id: &str) -> BTreeSet<String> {
        log_step(&self.lookups, "ids");
        let mut ids = lock(&self.aliases)
            .get(agent_id)
            .cloned()
            .unwrap_or_default();
        ids.insert(agent_id.to_owned());
        ids
    }
}

#[derive(Clone)]
pub(super) struct FakeRows {
    rows: Arc<Mutex<BTreeMap<String, KnownAgent>>>,
    failure: Arc<Mutex<Option<String>>>,
    lookups: Trace,
}

impl FakeRows {
    fn new(lookups: &Trace) -> Self {
        Self {
            rows: Arc::new(Mutex::new(BTreeMap::new())),
            failure: Arc::new(Mutex::new(None)),
            lookups: Arc::clone(lookups),
        }
    }

    pub(super) fn add(&self, agent: KnownAgent) {
        lock(&self.rows).insert(agent.agent_id.clone(), agent);
    }

    pub(super) fn fail_with(&self, message: &str) {
        *lock(&self.failure) = Some(message.to_owned());
    }
}

#[async_trait]
impl AgentRowLookup for FakeRows {
    async fn agent_row(&self, agent_id: &str) -> Result<Option<KnownAgent>, PortFailure> {
        log_step(&self.lookups, "row");
        let failure = lock(&self.failure).clone();
        if let Some(message) = failure {
            return Err(PortFailure {
                source: Box::new(FakeFailure { message }),
            });
        }
        Ok(lock(&self.rows).get(agent_id).cloned())
    }
}

#[derive(Clone)]
pub(super) struct FakeGroups {
    groups: Arc<Mutex<Option<Vec<String>>>>,
    steps: Trace,
}

impl FakeGroups {
    fn new(steps: &Trace) -> Self {
        Self {
            groups: Arc::new(Mutex::new(None)),
            steps: Arc::clone(steps),
        }
    }

    pub(super) fn seed(&self, groups: &[&str]) {
        *lock(&self.groups) = Some(groups.iter().map(|group| (*group).to_owned()).collect());
    }

    pub(super) fn current(&self) -> Option<Vec<String>> {
        lock(&self.groups).clone()
    }
}

impl PublicGroupStore for FakeGroups {
    fn groups(&self) -> Option<Vec<String>> {
        self.current()
    }

    fn set_groups(&self, groups: Vec<String>) {
        log_step(&self.steps, "memory");
        *lock(&self.groups) = Some(groups);
    }
}

#[derive(Clone)]
pub(super) struct FakeConfig {
    saved: Arc<Mutex<Option<Vec<String>>>>,
    failure: Arc<Mutex<Option<String>>>,
    steps: Trace,
}

impl FakeConfig {
    fn new(steps: &Trace) -> Self {
        Self {
            saved: Arc::new(Mutex::new(None)),
            failure: Arc::new(Mutex::new(None)),
            steps: Arc::clone(steps),
        }
    }

    pub(super) fn fail_with(&self, message: &str) {
        *lock(&self.failure) = Some(message.to_owned());
    }

    pub(super) fn saved(&self) -> Option<Vec<String>> {
        lock(&self.saved).clone()
    }
}

#[async_trait]
impl PublicGroupConfigWriter for FakeConfig {
    async fn save_public_groups(&self, groups: &[String]) -> Result<(), PortFailure> {
        log_step(&self.steps, "config");
        let failure = lock(&self.failure).clone();
        if let Some(message) = failure {
            return Err(PortFailure {
                source: Box::new(FakeFailure { message }),
            });
        }
        *lock(&self.saved) = Some(groups.to_vec());
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct FakeTrigger {
    result: Arc<Mutex<AgentKillSwitchResult>>,
    fired: Arc<Mutex<Vec<(String, AgentKillSwitchConfig)>>>,
    steps: Trace,
}

impl FakeTrigger {
    fn new(steps: &Trace) -> Self {
        Self {
            result: Arc::new(Mutex::new(succeeded_result())),
            fired: Arc::new(Mutex::new(Vec::new())),
            steps: Arc::clone(steps),
        }
    }

    pub(super) fn answer(&self, result: AgentKillSwitchResult) {
        *lock(&self.result) = result;
    }

    pub(super) fn fired(&self) -> Vec<(String, AgentKillSwitchConfig)> {
        lock(&self.fired).clone()
    }
}

#[async_trait]
impl KillSwitchTrigger for FakeTrigger {
    async fn fire(&self, agent_id: &str, config: &AgentKillSwitchConfig) -> AgentKillSwitchResult {
        log_step(&self.steps, "fire");
        lock(&self.fired).push((agent_id.to_owned(), config.clone()));
        // fire_kill_switch builds its result with the id the route passed, so the fake names it too
        let mut result = lock(&self.result).clone();
        result.agent_id = agent_id.to_owned();
        result
    }
}

#[derive(Clone)]
pub(super) struct FakeAudit {
    records: Arc<Mutex<Vec<AuditLogRecord>>>,
    steps: Trace,
}

impl FakeAudit {
    fn new(steps: &Trace) -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            steps: Arc::clone(steps),
        }
    }

    pub(super) fn records(&self) -> Vec<AuditLogRecord> {
        lock(&self.records).clone()
    }
}

#[async_trait]
impl KillSwitchAuditWriter for FakeAudit {
    async fn write(&self, record: AuditLogRecord) {
        log_step(&self.steps, "audit");
        lock(&self.records).push(record);
    }
}

pub(super) struct FixedClock(pub(super) DateTime<FixedOffset>);

impl FixedClock {
    pub(super) fn at(text: &str) -> Self {
        Self(DateTime::parse_from_rfc3339(text).unwrap())
    }
}

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

#[derive(Clone)]
pub(super) struct Fixture {
    pub(super) registry: FakeRegistry,
    pub(super) rows: FakeRows,
    pub(super) groups: FakeGroups,
    pub(super) config: FakeConfig,
    pub(super) trigger: FakeTrigger,
    pub(super) audit: FakeAudit,
    pub(super) steps: Trace,
    pub(super) lookups: Trace,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let steps = new_trace();
        let lookups = new_trace();
        let registry = FakeRegistry::new(&lookups);
        let rows = FakeRows::new(&lookups);
        let groups = FakeGroups::new(&steps);
        let config = FakeConfig::new(&steps);
        let trigger = FakeTrigger::new(&steps);
        let audit = FakeAudit::new(&steps);
        Self {
            registry,
            rows,
            groups,
            config,
            trigger,
            audit,
            steps,
            lookups,
        }
    }

    pub(super) fn state(&self) -> AgentAdminState {
        AgentAdminState {
            database: Some(Arc::new(self.rows.clone())),
            registry: Arc::new(self.registry.clone()),
            public_groups: Arc::new(self.groups.clone()),
            config: Arc::new(self.config.clone()),
            kill_switch: Arc::new(self.trigger.clone()),
            audit: Arc::new(self.audit.clone()),
            clock: Arc::new(FixedClock::at("2026-01-02T03:04:05+00:00")),
            new_id: Arc::new(|| String::from("audit-1")),
            litellm_proxy_admin_name: Some(String::from("default_user_id")),
        }
    }

    pub(super) fn state_without_database(&self) -> AgentAdminState {
        let mut state = self.state();
        state.database = None;
        state
    }
}

pub(super) fn key(role: &str) -> UserAPIKeyAuth {
    key_with_user(role, Some("user-1"))
}

pub(super) fn key_with_user(role: &str, user_id: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert(String::from("user_role"), JsonData::String(role.to_owned()));
    fields.insert(
        String::from("api_key"),
        JsonData::String(String::from("sk-test")),
    );
    if let Some(id) = user_id {
        fields.insert(String::from("user_id"), JsonData::String(id.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn caller(role: &str) -> AuthContext {
    context(Some(key(role)), "admin-user")
}

pub(super) fn caller_with_user(role: &str, user_id: Option<&str>) -> AuthContext {
    context(Some(key_with_user(role, user_id)), "admin-user")
}

pub(super) fn master_key_caller() -> AuthContext {
    master_caller()
}

fn context(source_key: Option<UserAPIKeyAuth>, subject: &str) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
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
            source_key: SourceKey(source_key),
        },
    }
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub(super) fn detail(&self) -> TestDetail {
        self.json::<TestErrorBody>().detail
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(super) enum TestDetail {
    Text(String),
    Object { error: String },
}

#[derive(Debug, Deserialize)]
pub(super) struct TestErrorBody {
    pub(super) detail: TestDetail,
}

pub(super) async fn post(state: AgentAdminState, uri: &str, caller: &AuthContext) -> Reply {
    send(state, uri, Some(caller), Body::empty()).await
}

pub(super) async fn post_json(
    state: AgentAdminState,
    uri: &str,
    caller: &AuthContext,
    body: &str,
) -> Reply {
    send(state, uri, Some(caller), Body::from(body.to_owned())).await
}

pub(super) async fn post_anonymous(state: AgentAdminState, uri: &str) -> Reply {
    send(state, uri, None, Body::empty()).await
}

async fn send(
    state: AgentAdminState,
    uri: &str,
    caller: Option<&AuthContext>,
    body: Body,
) -> Reply {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap();
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    let response = agent_admin_routes(state).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        body: bytes.to_vec(),
    }
}
