mod auth_wire;
pub mod caller;
pub mod kill_switch_url;
mod tests;

use auth_wire::ExposedSecret;
use kill_switch_url::KillSwitchUrl;

use crate::budget_snapshots::lax_datetime::LaxDateTime;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ApiKeyHeader(String);

impl Default for ApiKeyHeader {
    fn default() -> Self {
        Self(String::from("x-api-key"))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentKillSwitchAuth {
    Bearer {
        #[serde(
            serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
        )]
        token: SecretString,
    },
    ApiKey {
        #[serde(default)]
        header_name: ApiKeyHeader,
        #[serde(
            serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
        )]
        api_key: SecretString,
    },
    Basic {
        username: String,
        #[serde(
            serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
        )]
        password: SecretString,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum AgentKillSwitchMethod {
    #[default]
    Post,
    Put,
    Patch,
    Delete,
    Get,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct RequiredScopes(pub Vec<String>);

impl Default for RequiredScopes {
    fn default() -> Self {
        Self(vec![String::from("user_impersonation")])
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentKillSwitchConfig {
    pub url: KillSwitchUrl,
    #[serde(default)]
    pub method: AgentKillSwitchMethod,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<BTreeMap<String, Box<RawValue>>>,
    #[serde(default)]
    pub auth: Option<AgentKillSwitchAuth>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentExecutionMode {
    #[default]
    Autonomous,
    Delegated,
    Both,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentIdentityBinding {
    pub agent_id: String,
    #[serde(default)]
    pub active: PolicyFlag,
    pub provider: IdentityProvider,
    pub tenant_id: String,
    pub client_id: String,
    #[serde(default)]
    pub service_principal_id: Option<String>,
    pub issuer: String,
    #[serde(default)]
    pub required_roles: Vec<String>,
    #[serde(default)]
    pub required_scopes: RequiredScopes,
    pub revision: String,
    #[serde(default)]
    pub last_authenticated_at: Option<LaxDateTime>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityProvider {
    MicrosoftEntra,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManagedAgentContext {
    pub agent_id: String,
    #[serde(default)]
    pub binding_revision: Option<String>,
    pub mode: ManagedAgentMode,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub subject_oid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentMode {
    Autonomous,
    Delegated,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentKeySummary {
    pub token: String,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub key_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentResponse {
    #[serde(default)]
    pub identity: Option<AgentIdentityBinding>,
    #[serde(default)]
    pub identity_managed: bool,
    #[serde(default)]
    pub enabled: PolicyFlag,
    #[serde(default)]
    pub execution_mode: AgentExecutionMode,
    #[serde(default)]
    pub jwt_auth_configured: bool,
    pub agent_id: String,
    pub agent_name: String,
    #[serde(default)]
    pub litellm_params: Option<BTreeMap<String, Box<RawValue>>>,
    pub agent_card_params: BTreeMap<String, Box<RawValue>>,
    #[serde(default)]
    pub object_permission: Option<BTreeMap<String, Box<RawValue>>>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub session_tpm_limit: Option<i64>,
    #[serde(default)]
    pub session_rpm_limit: Option<i64>,
    #[serde(default)]
    pub static_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_headers: Option<Vec<String>>,
    #[serde(default)]
    pub access_group_ids: Option<Vec<String>>,
    #[serde(default)]
    pub kill_switch: Option<AgentKillSwitchConfig>,
    #[serde(default)]
    pub keys: Option<Vec<AgentKeySummary>>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub search_score: Option<Decimal>,
    #[serde(default)]
    pub created_at: Option<LaxDateTime>,
    #[serde(default)]
    pub updated_at: Option<LaxDateTime>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct PolicyFlag(pub bool);

impl Default for PolicyFlag {
    fn default() -> Self {
        Self(true)
    }
}
