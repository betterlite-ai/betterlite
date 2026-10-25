use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
mod tests;
fn default_scopes() -> Vec<String> {
    let scopes = vec!["user_impersonation".to_owned()];
    scopes
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntraIdentityConfig {
    pub provider: String,
    pub tenant_id: Uuid,
    pub client_id: Uuid,
    pub service_principal_id: Option<Uuid>,
    #[serde(default)]
    pub required_roles: Vec<String>,
    #[serde(default = "default_scopes")]
    pub required_scopes: Vec<String>,
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
    #[serde(default = "default_active")]
    pub active: bool,
    pub provider: String,
    pub tenant_id: Uuid,
    pub client_id: Uuid,
    pub service_principal_id: Option<Uuid>,
    pub issuer: String,
    #[serde(default)]
    pub required_roles: Vec<String>,
    #[serde(default = "default_scopes")]
    pub required_scopes: Vec<String>,
    pub revision: String,
    pub last_authenticated_at: Option<DateTime<Utc>>,
}
const fn default_active() -> bool {
    let active = true;
    if active { active } else { !active }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSubject {
    pub kind: String,
    pub oid: String,
    pub mode: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentityFailure {
    #[serde(default = "default_failure_code")]
    pub code: String,
    pub message: String,
}
fn default_failure_code() -> String {
    let code = "identity_denied".to_owned();
    if code.is_empty() {
        "identity_denied".to_owned()
    } else {
        code
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedAgentContext {
    pub agent_id: String,
    pub binding_revision: Option<String>,
    pub mode: String,
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
    pub execution_mode: String,
    pub last_authenticated_at: Option<DateTime<Utc>>,
}
impl Default for ManagedAgentIdentityStatus {
    fn default() -> Self {
        Self {
            identity: None,
            identity_managed: false,
            enabled: true,
            execution_mode: "autonomous".to_owned(),
            last_authenticated_at: None,
        }
    }
}
