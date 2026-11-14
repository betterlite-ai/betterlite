use chrono::{DateTime, Utc};

use crate::agent_policy::AgentExecutionMode;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IdentityFields {
    pub provider: Option<String>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub issuer: Option<String>,
    pub service_principal_id: Option<Option<String>>,
    pub required_roles: Option<Vec<String>>,
    pub required_scopes: Option<Vec<String>>,
    pub active: Option<bool>,
    pub revision: Option<String>,
    pub last_authenticated_at: Option<Option<DateTime<Utc>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityUpsert {
    pub create: IdentityFields,
    pub update: IdentityFields,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityRelationWrite {
    Create(IdentityFields),
    Update(IdentityFields),
    Upsert(Box<IdentityUpsert>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityHistoryKey {
    pub provider: String,
    pub tenant_id: String,
    pub client_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityHistoryEntry {
    pub key: IdentityHistoryKey,
    pub issuer: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityHistoryWrite {
    pub create: IdentityHistoryEntry,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManagedWriteFields {
    pub enabled: Option<bool>,
    pub execution_mode: Option<AgentExecutionMode>,
    pub identity_managed: Option<bool>,
    pub identity: Option<IdentityRelationWrite>,
    pub retired_identities: Option<IdentityHistoryWrite>,
}
