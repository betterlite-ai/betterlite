use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod json;
mod tests;
mod validation;
use json::{AgentFlag, AgentJson, SecurityRequirements};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    pub organization: Option<String>,
    pub url: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExtension {
    pub uri: Option<String>,
    pub description: Option<String>,
    pub required: Option<bool>,
    pub params: Option<BTreeMap<String, AgentJson>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: Option<bool>,
    pub push_notifications: Option<bool>,
    pub state_transition_history: Option<bool>,
    pub extensions: Option<Vec<AgentExtension>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SecurityScheme {
    ApiKey {
        #[serde(rename = "in")]
        location: String,
        name: String,
        description: Option<String>,
    },
    Http {
        scheme: String,
        bearer_format: Option<String>,
        description: Option<String>,
    },
    MutualTls {
        description: Option<String>,
    },
    Oauth2 {
        flows: BTreeMap<String, AgentJson>,
        oauth2_metadata_url: Option<String>,
        description: Option<String>,
    },
    OpenIdConnect {
        open_id_connect_url: String,
        description: Option<String>,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub examples: Option<Vec<String>>,
    pub input_modes: Option<Vec<String>>,
    pub output_modes: Option<Vec<String>>,
    pub security: Option<SecurityRequirements>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInterface {
    pub url: Option<String>,
    pub transport: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCardSignature {
    pub protected: Option<String>,
    pub signature: Option<String>,
    pub header: Option<BTreeMap<String, AgentJson>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub protocol_version: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub version: Option<String>,
    pub capabilities: Option<AgentCapabilities>,
    pub default_input_modes: Option<Vec<String>>,
    pub default_output_modes: Option<Vec<String>>,
    pub skills: Option<Vec<AgentSkill>>,
    pub preferred_transport: Option<String>,
    pub additional_interfaces: Option<Vec<AgentInterface>>,
    pub icon_url: Option<String>,
    pub provider: Option<AgentProvider>,
    pub documentation_url: Option<String>,
    pub security_schemes: Option<BTreeMap<String, SecurityScheme>>,
    pub security: Option<SecurityRequirements>,
    #[serde(rename = "supportsAuthenticatedExtendedCard")]
    pub supports_authenticated_card: Option<bool>,
    pub signatures: Option<Vec<AgentCardSignature>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AugmentedAgentCard {
    #[serde(flatten)]
    pub card: AgentCard,
    pub is_public: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentObjectPermission {
    pub mcp_servers: Option<Vec<String>>,
    pub mcp_access_groups: Option<Vec<String>>,
    pub mcp_toolsets: Option<Vec<String>>,
    pub mcp_tool_permissions: Option<BTreeMap<String, Vec<String>>>,
    pub models: Option<Vec<String>>,
    pub agents: Option<Vec<String>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentKillSwitchAuth {
    Bearer {
        token: String,
    },
    ApiKey {
        header_name: String,
        api_key: String,
    },
    Basic {
        username: String,
        password: String,
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
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentKillSwitchConfig {
    #[serde(deserialize_with = "validation::absolute_http_url")]
    pub url: String,
    #[serde(default)]
    pub method: AgentKillSwitchMethod,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    pub body: Option<BTreeMap<String, AgentJson>>,
    pub auth: Option<AgentKillSwitchAuth>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentKillSwitchResult {
    pub agent_id: String,
    pub url: String,
    pub method: AgentKillSwitchMethod,
    pub status_code: Option<u16>,
    pub response_body: Option<String>,
    pub error: Option<String>,
}
impl AgentKillSwitchResult {
    #[must_use]
    pub fn succeeded(&self) -> bool {
        let status = self.status_code;
        status.is_some_and(|value| (200..300).contains(&value))
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentConfig {
    pub identity: Option<BTreeMap<String, AgentJson>>,
    pub enabled: Option<bool>,
    pub execution_mode: Option<String>,
    pub agent_name: String,
    pub agent_card_params: Option<AgentCard>,
    pub litellm_params: Option<BTreeMap<String, AgentJson>>,
    pub object_permission: Option<AgentObjectPermission>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub session_tpm_limit: Option<i64>,
    pub session_rpm_limit: Option<i64>,
    pub static_headers: Option<BTreeMap<String, String>>,
    pub extra_headers: Option<Vec<String>>,
    pub access_group_ids: Option<Vec<String>>,
    pub kill_switch: Option<AgentKillSwitchConfig>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PatchAgentRequest {
    pub identity: Option<BTreeMap<String, AgentJson>>,
    pub enabled: Option<bool>,
    pub execution_mode: Option<String>,
    pub agent_name: Option<String>,
    pub agent_card_params: Option<AgentCard>,
    pub litellm_params: Option<BTreeMap<String, AgentJson>>,
    pub object_permission: Option<AgentObjectPermission>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub session_tpm_limit: Option<i64>,
    pub session_rpm_limit: Option<i64>,
    pub static_headers: Option<BTreeMap<String, String>>,
    pub extra_headers: Option<Vec<String>>,
    pub access_group_ids: Option<Vec<String>>,
    pub kill_switch: Option<AgentKillSwitchConfig>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentCaller {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentKeySummary {
    pub token: String,
    pub key_alias: Option<String>,
    pub key_name: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentResponse {
    pub identity: Option<BTreeMap<String, AgentJson>>,
    pub identity_managed: AgentFlag,
    pub enabled: AgentFlag,
    pub execution_mode: String,
    pub jwt_auth_configured: AgentFlag,
    pub agent_id: String,
    pub agent_name: String,
    pub litellm_params: Option<BTreeMap<String, AgentJson>>,
    pub agent_card_params: BTreeMap<String, AgentJson>,
    pub object_permission: Option<BTreeMap<String, AgentJson>>,
    pub spend: Option<Decimal>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub session_tpm_limit: Option<i64>,
    pub session_rpm_limit: Option<i64>,
    pub static_headers: Option<BTreeMap<String, String>>,
    pub extra_headers: Option<Vec<String>>,
    pub access_group_ids: Option<Vec<String>>,
    pub kill_switch: Option<AgentKillSwitchConfig>,
    pub keys: Option<Vec<AgentKeySummary>>,
    pub search_score: Option<Decimal>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
}
impl Default for AgentResponse {
    fn default() -> Self {
        Self {
            identity: None,
            identity_managed: false,
            enabled: true,
            execution_mode: "autonomous".to_owned(),
            jwt_auth_configured: false,
            agent_id: String::new(),
            agent_name: String::new(),
            litellm_params: None,
            agent_card_params: BTreeMap::new(),
            object_permission: None,
            spend: None,
            tpm_limit: None,
            rpm_limit: None,
            session_tpm_limit: None,
            session_rpm_limit: None,
            static_headers: None,
            extra_headers: None,
            access_group_ids: None,
            kill_switch: None,
            keys: None,
            search_score: None,
            created_at: None,
            updated_at: None,
            created_by: None,
            updated_by: None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentMakePublicResponse {
    pub message: String,
    pub public_agent_groups: Vec<String>,
    pub updated_by: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MakeAgentsPublicRequest {
    pub agent_ids: Vec<String>,
}
