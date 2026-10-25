use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod tests;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityProvider {
    MicrosoftEntra,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Autonomous,
    Delegated,
    Both,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    User,
    ServicePrincipal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubjectMode {
    Interactive,
    Managed,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    #[default]
    IdentityDenied,
    PolicyUnavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RequiredScopes(pub Vec<String>);

impl RequiredScopes {
    fn default_value() -> Self {
        let scope = String::from("user_impersonation");
        let scopes = vec![scope];
        Self(scopes)
    }
}

impl Default for RequiredScopes {
    fn default() -> Self {
        Self::default_value()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntraIdentityConfig {
    pub provider: IdentityProvider,
    pub tenant_id: Uuid,
    pub client_id: Uuid,
    pub service_principal_id: Option<Uuid>,
    #[serde(default)]
    pub required_roles: Vec<String>,
    #[serde(default = "RequiredScopes::default_value")]
    pub required_scopes: RequiredScopes,
}

impl EntraIdentityConfig {
    #[must_use]
    pub fn issuer(&self) -> String {
        let tenant = self.tenant_id;
        format!("https://login.microsoftonline.com/{tenant}/v2.0")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentityBinding {
    pub agent_id: String,
    #[serde(default)]
    pub active: ActiveState,
    pub provider: IdentityProvider,
    pub tenant_id: Uuid,
    pub client_id: Uuid,
    pub service_principal_id: Option<Uuid>,
    pub issuer: String,
    #[serde(default)]
    pub required_roles: Vec<String>,
    #[serde(default = "RequiredScopes::default_value")]
    pub required_scopes: RequiredScopes,
    pub revision: String,
    pub last_authenticated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ActiveState(pub bool);

impl Default for ActiveState {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSubject {
    pub kind: SubjectKind,
    pub oid: String,
    pub mode: SubjectMode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentityFailure {
    #[serde(default)]
    pub code: FailureCode,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedAgentContext {
    pub agent_id: String,
    pub binding_revision: Option<String>,
    pub mode: ExecutionMode,
    pub user_id: Option<String>,
    pub subject_oid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedHumanSubject {
    pub issuer: String,
    pub tenant_id: String,
    pub oid: String,
    pub user_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MicrosoftInteractiveSubject {
    pub issuer: String,
    pub tenant_id: String,
    pub oid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct ManagedAgentIdentityStatus {
    pub identity: Option<AgentIdentityBinding>,
    pub identity_managed: bool,
    pub enabled: bool,
    pub execution_mode: ExecutionMode,
    pub last_authenticated_at: Option<DateTime<Utc>>,
}

impl Default for ManagedAgentIdentityStatus {
    fn default() -> Self {
        Self {
            identity: None,
            identity_managed: false,
            enabled: true,
            execution_mode: ExecutionMode::Autonomous,
            last_authenticated_at: None,
        }
    }
}
