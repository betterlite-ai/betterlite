mod tests;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub const CALLER_USER_ID_HEADER: &str = "x-litellm-user-id";
pub const CALLER_TEAM_ID_HEADER: &str = "x-litellm-team-id";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentKillSwitchAuth {
    Bearer {
        token: SecretValue,
    },
    ApiKey {
        #[serde(default = "default_api_key_header")]
        header_name: String,
        api_key: SecretValue,
    },
    Basic {
        username: String,
        password: SecretValue,
    },
}

pub mod secret_value;
use secret_value::SecretValue;

impl AgentKillSwitchAuth {
    #[must_use]
    pub const fn secret_value(&self) -> &str {
        match self {
            Self::Bearer { token } => token.expose(),
            Self::ApiKey { api_key, .. } => api_key.expose(),
            Self::Basic { password, .. } => password.expose(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum AgentKillSwitchMethod {
    #[default]
    #[serde(rename = "POST")]
    Post,
    #[serde(rename = "PUT")]
    Put,
    #[serde(rename = "PATCH")]
    Patch,
    #[serde(rename = "DELETE")]
    Delete,
    #[serde(rename = "GET")]
    Get,
}

fn default_api_key_header() -> String {
    let mut header = String::new();
    header.push_str("x-api-key");
    header
}

fn default_required_scopes() -> Vec<String> {
    let scope = String::from("user_impersonation");
    vec![scope]
}

const fn default_active() -> bool {
    match Some(true) {
        Some(active) => active,
        None => false,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentKillSwitchConfig {
    pub url: String,
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
pub struct AgentCaller {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
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
pub struct AgentIdentityBinding {
    pub agent_id: String,
    #[serde(default = "default_active")]
    pub active: bool,
    pub provider: String,
    pub tenant_id: String,
    pub client_id: String,
    #[serde(default)]
    pub service_principal_id: Option<String>,
    pub issuer: String,
    #[serde(default)]
    pub required_roles: Vec<String>,
    #[serde(default = "default_required_scopes")]
    pub required_scopes: Vec<String>,
    pub revision: String,
    #[serde(default)]
    pub last_authenticated_at: Option<DateTime<Utc>>,
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
pub struct AgentResponse {
    #[serde(default)]
    pub identity: Option<AgentIdentityBinding>,
    #[serde(default)]
    pub identity_managed: PolicyFlag,
    #[serde(default)]
    pub enabled: PolicyFlag,
    #[serde(default)]
    pub execution_mode: AgentExecutionMode,
    #[serde(default)]
    pub jwt_auth_configured: PolicyFlag,
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
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
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
