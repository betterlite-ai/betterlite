use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
mod tests;

macro_rules! record {
    ($name:ident { $($fields:tt)* }) => {
        #[derive(Clone, Debug, Deserialize, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $($fields)* }
    };
}
macro_rules! identity_enum {
    ($name:ident { $first:ident $(, $variant:ident)* }) => {
        #[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { #[default] $first, $($variant,)* }
    };
}
identity_enum! { IdentityProvider { MicrosoftEntra } }
identity_enum! { ExecutionMode { Autonomous, Delegated, Both } }
identity_enum! { SubjectKind { Application, DelegatedSubject } }
identity_enum! { SubjectMode { Autonomous, Delegated } }
identity_enum! { FailureCode { IdentityDenied, PolicyUnavailable } }
fn delegated_scopes() -> Vec<String> {
    let scope = String::from("user_impersonation");
    vec![scope]
}
record! { EntraIdentityConfig {
    pub provider: IdentityProvider,
    pub tenant_id: Uuid,
    pub client_id: Uuid,
    pub service_principal_id: Option<Uuid>,
    #[serde(default)] pub required_roles: Vec<String>,
    #[serde(default = "delegated_scopes")] pub required_scopes: Vec<String>,
}}
impl EntraIdentityConfig {
    #[must_use]
    pub fn issuer(&self) -> String {
        let tenant = self.tenant_id;
        format!("https://login.microsoftonline.com/{tenant}/v2.0")
    }
}
record! { AgentIdentityBinding {
    pub agent_id: String,
    #[serde(default)] pub active: ActiveState,
    pub provider: IdentityProvider,
    pub tenant_id: Uuid,
    pub client_id: Uuid,
    pub service_principal_id: Option<Uuid>,
    pub issuer: String,
    #[serde(default)] pub required_roles: Vec<String>,
    #[serde(default = "delegated_scopes")] pub required_scopes: Vec<String>,
    pub revision: String,
    pub last_authenticated_at: Option<DateTime<Utc>>,
}}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ActiveState(pub bool);
impl Default for ActiveState {
    fn default() -> Self {
        Self(true)
    }
}
record! { AgentSubject {
    pub kind: SubjectKind,
    pub oid: String,
    pub mode: SubjectMode,
}}
record! { AgentIdentityFailure {
    #[serde(default)] pub code: FailureCode,
    pub message: String,
}}
record! { ManagedAgentContext {
    pub agent_id: String,
    pub binding_revision: Option<String>,
    pub mode: SubjectMode,
    pub user_id: Option<String>,
    pub subject_oid: Option<String>,
}}
record! { VerifiedHumanSubject {
    pub issuer: String,
    pub tenant_id: String,
    pub oid: String,
    pub user_id: String,
}}
record! { MicrosoftInteractiveSubject {
    pub issuer: String,
    pub tenant_id: String,
    pub oid: String,
}}
record! { ManagedAgentIdentityStatus {
    pub identity: Option<AgentIdentityBinding>,
    #[serde(default)] pub identity_managed: bool,
    #[serde(default)] pub enabled: ActiveState,
    #[serde(default)] pub execution_mode: ExecutionMode,
    pub last_authenticated_at: Option<DateTime<Utc>>,
}}
