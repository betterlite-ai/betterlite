use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use super::error::AgentStoreError;
use crate::agent_policy::AgentResponse;

// the database vocabulary the identity store writes and matches on
pub(super) const HUMAN_KIND: &str = "human";
pub(super) const MICROSOFT_ENTRA_PROVIDER: &str = "microsoft_entra";
pub(super) const SSO_INTERACTIVE: &str = "sso_interactive";

pub type FindResult<Record> = Result<Option<Record>, AgentStoreError>;

// Prisma's provider_tenant_id_client_id input, shared by the identity and retired tables
#[derive(Clone, Copy, Debug)]
pub struct BindingKey<'key> {
    pub provider: &'key str,
    pub tenant_id: &'key str,
    pub client_id: &'key str,
}

// Prisma's issuer_tenant_id_oid input of the verified-subject table
#[derive(Clone, Copy, Debug)]
pub struct SubjectKey<'key> {
    pub issuer: &'key str,
    pub tenant_id: &'key str,
    pub oid: &'key str,
}

// find_unique returns the composite-key row; only its agent_id reaches a caller
#[derive(Clone, Debug)]
pub struct IdentityRecord {
    pub agent_id: String,
}

// the verified-subject row in the columns LiteLLM's Prisma model carries
#[derive(Clone, Debug, Deserialize)]
pub struct VerifiedSubjectRow {
    pub subject_id: String,
    pub issuer: String,
    pub tenant_id: String,
    pub oid: String,
    pub kind: String,
    pub user_id: Option<String>,
    pub verified_via: String,
    pub verified_at: NaiveDateTime,
}

// presence of this row is what retires the binding, so its primary key is all the store reads
#[derive(Clone, Debug)]
pub struct RetiredIdentityRecord {
    pub binding_id: String,
}

// presence of this row is what marks an agent as retired
#[derive(Clone, Debug)]
pub struct RetiredAgentRecord {
    pub original_agent_id: String,
}

// the nested agent relation filter of update_many, agent: {is: {enabled: true, identity_managed: true}}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentStateFilter {
    pub enabled: bool,
    pub identity_managed: bool,
}

// the where clause of update_many: the binding must still match its revision and be active
#[derive(Clone, Copy, Debug)]
pub struct AuthenticationFilter<'filter> {
    pub agent_id: &'filter str,
    pub revision: &'filter str,
    pub active: bool,
    pub agent: AgentStateFilter,
}

// the upsert's create data; the update data is empty, so a retry never rewrites an enrollment
#[derive(Clone, Copy, Debug)]
pub struct InteractiveEnrollment<'enrollment> {
    pub issuer: &'enrollment str,
    pub tenant_id: &'enrollment str,
    pub oid: &'enrollment str,
    pub user_id: &'enrollment str,
    pub verified_via: &'enrollment str,
}

#[async_trait]
pub trait AgentRecords: Send + Sync {
    async fn find_unique(&self, agent_id: &str) -> FindResult<AgentResponse>;
}

#[async_trait]
pub trait IdentityRecords: Send + Sync {
    async fn find_unique(&self, key: &BindingKey<'_>) -> FindResult<IdentityRecord>;

    async fn update_many(
        &self,
        filter: &AuthenticationFilter<'_>,
        at: DateTime<Utc>,
    ) -> Result<u64, AgentStoreError>;
}

#[async_trait]
pub trait VerifiedSubjectRecords: Send + Sync {
    async fn find_unique(&self, key: &SubjectKey<'_>) -> FindResult<VerifiedSubjectRow>;

    async fn upsert(
        &self,
        enrollment: &InteractiveEnrollment<'_>,
    ) -> Result<VerifiedSubjectRow, AgentStoreError>;
}

#[async_trait]
pub trait RetiredIdentityRecords: Send + Sync {
    async fn find_unique(&self, key: &BindingKey<'_>) -> FindResult<RetiredIdentityRecord>;
}

#[async_trait]
pub trait RetiredAgentRecords: Send + Sync {
    async fn find_unique(&self, agent_id: &str) -> FindResult<RetiredAgentRecord>;
}
