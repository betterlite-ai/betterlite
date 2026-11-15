use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_db::identity_rows::Payload;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

use crate::agent_policy::{AgentResponse, ManagedAgentContext, ManagedAgentMode};
use crate::agent_store::AgentIdentityStore;
use crate::agent_store::cache::ManagementObjectCache;
use crate::agent_store::error::AgentStoreError;
use crate::agent_store::records::{
    AgentRecords, AgentStateFilter, AuthenticationFilter, IdentityRecord, IdentityRecords,
    InteractiveEnrollment, RetiredAgentRecord, RetiredAgentRecords, RetiredIdentityRecord,
    RetiredIdentityRecords, VerifiedSubjectRecords, VerifiedSubjectRow,
};
use crate::identity::MicrosoftInteractiveSubject;

const APPLICATION_CLAIMS: &str =
    r#"{"iss":"issuer-1","tid":"tenant-1","azp":"client-1","oid":"oid-1"}"#;

const DELEGATED_CLAIMS: &str = r#"{"iss":"issuer-1","tid":"tenant-1","azp":"client-2","oid":"oid-2","scp":"user_impersonation"}"#;

const AUTONOMOUS_AGENT: &str = r#"{
    "agent_id": "agent-1",
    "agent_name": "agent one",
    "agent_card_params": {"name": "agent one"},
    "object_permission": {"mcp_servers": ["mcp-1"]},
    "identity_managed": true,
    "enabled": true,
    "execution_mode": "autonomous",
    "identity": {
        "agent_id": "agent-1",
        "active": true,
        "provider": "microsoft_entra",
        "tenant_id": "tenant-1",
        "client_id": "client-1",
        "service_principal_id": "oid-1",
        "issuer": "issuer-1",
        "required_roles": [],
        "required_scopes": ["user_impersonation"],
        "revision": "rev-1"
    }
}"#;

const DELEGATED_AGENT: &str = r#"{
    "agent_id": "agent-2",
    "agent_name": "agent two",
    "agent_card_params": {},
    "identity_managed": true,
    "enabled": true,
    "execution_mode": "delegated",
    "identity": {
        "agent_id": "agent-2",
        "active": true,
        "provider": "microsoft_entra",
        "tenant_id": "tenant-1",
        "client_id": "client-2",
        "service_principal_id": "sp-1",
        "issuer": "issuer-1",
        "required_roles": [],
        "required_scopes": ["user_impersonation"],
        "revision": "rev-2"
    }
}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordedBindingKey {
    pub provider: String,
    pub tenant_id: String,
    pub client_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordedSubjectKey {
    pub issuer: String,
    pub tenant_id: String,
    pub oid: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FakeCall {
    Agent,
    Identity,
    Authentication,
    Subject,
    Enrollment,
    RetiredIdentity,
    RetiredAgent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordedAuthentication {
    pub agent_id: String,
    pub revision: String,
    pub active: bool,
    pub agent: AgentStateFilter,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordedEnrollment {
    pub issuer: String,
    pub tenant_id: String,
    pub oid: String,
    pub user_id: String,
    pub verified_via: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordedCacheWrite {
    pub key: String,
    pub value: Json,
    pub ttl: f64,
}

#[derive(Default)]
pub(crate) struct FakeState {
    pub agent: Option<AgentResponse>,
    pub identity: Option<IdentityRecord>,
    pub authentication_count: u64,
    pub subject: Option<VerifiedSubjectRow>,
    pub enrollment_row: Option<VerifiedSubjectRow>,
    pub retired_identity: Option<RetiredIdentityRecord>,
    pub retired_agent: Option<RetiredAgentRecord>,
    pub failure: Option<FakeCall>,
    pub agent_reads: Vec<String>,
    pub identity_reads: Vec<RecordedBindingKey>,
    pub authentications: Vec<RecordedAuthentication>,
    pub subject_reads: Vec<RecordedSubjectKey>,
    pub enrollments: Vec<RecordedEnrollment>,
    pub retired_identity_reads: Vec<RecordedBindingKey>,
    pub retired_agent_reads: Vec<String>,
}

#[derive(Default)]
pub(crate) struct FakeRecords {
    pub state: Mutex<FakeState>,
}

impl FakeRecords {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, FakeState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub(crate) fn store_failure() -> AgentStoreError {
    AgentStoreError::Database(DbError::NoDbConnected)
}

#[async_trait]
impl AgentRecords for FakeRecords {
    async fn find_unique(&self, agent_id: &str) -> Result<Option<AgentResponse>, AgentStoreError> {
        let mut state = self.state();
        state.agent_reads.push(agent_id.to_owned());
        if state.failure == Some(FakeCall::Agent) {
            return Err(store_failure());
        }
        Ok(state.agent.clone())
    }
}

#[async_trait]
impl IdentityRecords for FakeRecords {
    async fn find_unique(
        &self,
        provider: &str,
        tenant_id: &str,
        client_id: &str,
    ) -> Result<Option<IdentityRecord>, AgentStoreError> {
        let mut state = self.state();
        state.identity_reads.push(RecordedBindingKey {
            provider: provider.to_owned(),
            tenant_id: tenant_id.to_owned(),
            client_id: client_id.to_owned(),
        });
        if state.failure == Some(FakeCall::Identity) {
            return Err(store_failure());
        }
        Ok(state.identity.clone())
    }

    async fn update_many(
        &self,
        filter: &AuthenticationFilter<'_>,
        at: DateTime<Utc>,
    ) -> Result<u64, AgentStoreError> {
        let mut state = self.state();
        state.authentications.push(RecordedAuthentication {
            agent_id: filter.agent_id.to_owned(),
            revision: filter.revision.to_owned(),
            active: filter.active,
            agent: filter.agent,
            at,
        });
        if state.failure == Some(FakeCall::Authentication) {
            return Err(store_failure());
        }
        Ok(state.authentication_count)
    }
}

#[async_trait]
impl VerifiedSubjectRecords for FakeRecords {
    async fn find_unique(
        &self,
        issuer: &str,
        tenant_id: &str,
        oid: &str,
    ) -> Result<Option<VerifiedSubjectRow>, AgentStoreError> {
        let mut state = self.state();
        state.subject_reads.push(RecordedSubjectKey {
            issuer: issuer.to_owned(),
            tenant_id: tenant_id.to_owned(),
            oid: oid.to_owned(),
        });
        if state.failure == Some(FakeCall::Subject) {
            return Err(store_failure());
        }
        Ok(state.subject.clone())
    }

    async fn upsert(
        &self,
        enrollment: &InteractiveEnrollment<'_>,
    ) -> Result<VerifiedSubjectRow, AgentStoreError> {
        let mut state = self.state();
        state.enrollments.push(RecordedEnrollment {
            issuer: enrollment.issuer.to_owned(),
            tenant_id: enrollment.tenant_id.to_owned(),
            oid: enrollment.oid.to_owned(),
            user_id: enrollment.user_id.to_owned(),
            verified_via: enrollment.verified_via.to_owned(),
        });
        if state.failure == Some(FakeCall::Enrollment) {
            return Err(store_failure());
        }
        Ok(state
            .enrollment_row
            .clone()
            .expect("the test configures the upsert row"))
    }
}

#[async_trait]
impl RetiredIdentityRecords for FakeRecords {
    async fn find_unique(
        &self,
        provider: &str,
        tenant_id: &str,
        client_id: &str,
    ) -> Result<Option<RetiredIdentityRecord>, AgentStoreError> {
        let mut state = self.state();
        state.retired_identity_reads.push(RecordedBindingKey {
            provider: provider.to_owned(),
            tenant_id: tenant_id.to_owned(),
            client_id: client_id.to_owned(),
        });
        if state.failure == Some(FakeCall::RetiredIdentity) {
            return Err(store_failure());
        }
        Ok(state.retired_identity.clone())
    }
}

#[async_trait]
impl RetiredAgentRecords for FakeRecords {
    async fn find_unique(
        &self,
        agent_id: &str,
    ) -> Result<Option<RetiredAgentRecord>, AgentStoreError> {
        let mut state = self.state();
        state.retired_agent_reads.push(agent_id.to_owned());
        if state.failure == Some(FakeCall::RetiredAgent) {
            return Err(store_failure());
        }
        Ok(state.retired_agent.clone())
    }
}

#[derive(Default)]
pub(crate) struct FakeCacheState {
    pub value: Option<Json>,
    pub ttl: f64,
    pub writes: Vec<RecordedCacheWrite>,
}

#[derive(Default)]
pub(crate) struct FakeCache {
    pub state: Mutex<FakeCacheState>,
}

impl FakeCache {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, FakeCacheState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[async_trait]
impl ManagementObjectCache for FakeCache {
    async fn read(&self, _key: &str) -> Option<Json> {
        self.state().value.clone()
    }

    fn management_ttl(&self) -> f64 {
        self.state().ttl
    }

    async fn write(&self, key: &str, value: &Json, ttl: f64) {
        let mut state = self.state();
        state.writes.push(RecordedCacheWrite {
            key: key.to_owned(),
            value: value.clone(),
            ttl,
        });
    }
}

pub(crate) fn fixed_clock() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("the fixed instant")
}

pub(crate) fn autonomous_agent() -> AgentResponse {
    serde_json::from_str(AUTONOMOUS_AGENT).expect("the autonomous agent fixture parses")
}

pub(crate) fn delegated_agent() -> AgentResponse {
    serde_json::from_str(DELEGATED_AGENT).expect("the delegated agent fixture parses")
}

pub(crate) fn application_claims() -> Payload {
    serde_json::from_str(APPLICATION_CLAIMS).expect("the application claims parse")
}

pub(crate) fn delegated_claims() -> Payload {
    serde_json::from_str(DELEGATED_CLAIMS).expect("the delegated claims parse")
}

pub(crate) fn binding(agent_id: &str) -> IdentityRecord {
    IdentityRecord {
        agent_id: agent_id.to_owned(),
    }
}

pub(crate) fn human_row(
    kind: &str,
    verified_via: &str,
    user_id: Option<&str>,
) -> VerifiedSubjectRow {
    VerifiedSubjectRow {
        subject_id: String::from("subject-1"),
        issuer: String::from("issuer-1"),
        tenant_id: String::from("tenant-1"),
        oid: String::from("oid-2"),
        kind: kind.to_owned(),
        user_id: user_id.map(str::to_owned),
        verified_via: verified_via.to_owned(),
        verified_at: fixed_clock().naive_utc(),
    }
}

pub(crate) fn authentication_context(
    agent_id: &str,
    revision: Option<&str>,
) -> ManagedAgentContext {
    ManagedAgentContext {
        agent_id: agent_id.to_owned(),
        binding_revision: revision.map(str::to_owned),
        mode: ManagedAgentMode::Autonomous,
        user_id: None,
        subject_oid: None,
    }
}

pub(crate) fn microsoft_subject() -> MicrosoftInteractiveSubject {
    MicrosoftInteractiveSubject {
        issuer: String::from("issuer-1"),
        tenant_id: String::from("tenant-1"),
        oid: String::from("oid-2"),
    }
}

pub(crate) fn store(fake: &Arc<FakeRecords>) -> AgentIdentityStore {
    AgentIdentityStore {
        agents: Arc::clone(fake),
        identities: Arc::clone(fake),
        humans: Arc::clone(fake),
        retired: Some(Arc::clone(fake)),
        retired_agents: Some(Arc::clone(fake)),
        cache: None,
        clock: fixed_clock,
    }
}

pub(crate) fn store_without_retired(fake: &Arc<FakeRecords>) -> AgentIdentityStore {
    AgentIdentityStore {
        agents: Arc::clone(fake),
        identities: Arc::clone(fake),
        humans: Arc::clone(fake),
        retired: None,
        retired_agents: Some(Arc::clone(fake)),
        cache: None,
        clock: fixed_clock,
    }
}

pub(crate) fn store_without_retired_agents(fake: &Arc<FakeRecords>) -> AgentIdentityStore {
    AgentIdentityStore {
        agents: Arc::clone(fake),
        identities: Arc::clone(fake),
        humans: Arc::clone(fake),
        retired: Some(Arc::clone(fake)),
        retired_agents: None,
        cache: None,
        clock: fixed_clock,
    }
}

pub(crate) fn store_with_cache(
    fake: &Arc<FakeRecords>,
    cache: &Arc<FakeCache>,
) -> AgentIdentityStore {
    AgentIdentityStore {
        agents: Arc::clone(fake),
        identities: Arc::clone(fake),
        humans: Arc::clone(fake),
        retired: Some(Arc::clone(fake)),
        retired_agents: Some(Arc::clone(fake)),
        cache: Some(Arc::clone(cache)),
        clock: fixed_clock,
    }
}
