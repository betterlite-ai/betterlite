use betterlite_auth::identity::{AgentIdentityBinding, EntraIdentityConfig, ExecutionMode};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod json;
mod tests;
mod validation;
use json::{AgentBoolean, AgentJson, SecurityRequirements};

fn default_api_key_header() -> String {
    let value = "x-api-key";
    String::from(value)
}

const fn default_enabled() -> AgentBoolean {
    let value = true;
    AgentBoolean(value)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde_with::skip_serializing_none]
pub struct AgentProvider {
    pub organization: Option<String>,
    pub url: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde_with::skip_serializing_none]
pub struct AgentExtension {
    pub uri: Option<String>,
    pub description: Option<String>,
    pub required: Option<bool>,
    pub params: Option<BTreeMap<String, AgentJson>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde_with::skip_serializing_none]
pub struct AgentCapabilities {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub streaming: Option<Option<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_notifications: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_transition_history: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<AgentExtension>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SecurityScheme {
    ApiKey(SecurityApiKey),
    Http(SecurityHttp),
    MutualTls(SecurityMutualTls),
    Oauth2(SecurityOauth2),
    OpenIdConnect(SecurityOpenIdConnect),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityApiKey {
    #[serde(rename = "type")]
    pub scheme_type: String,
    #[serde(rename = "in")]
    pub location: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityHttp {
    #[serde(rename = "type")]
    pub scheme_type: String,
    pub scheme: String,
    pub bearer_format: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityMutualTls {
    #[serde(rename = "type")]
    pub scheme_type: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityOauth2 {
    #[serde(rename = "type")]
    pub scheme_type: String,
    pub flows: BTreeMap<String, AgentJson>,
    pub oauth2_metadata_url: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityOpenIdConnect {
    #[serde(rename = "type")]
    pub scheme_type: String,
    pub open_id_connect_url: String,
    pub description: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde_with::skip_serializing_none]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde_with::skip_serializing_none]
pub struct AgentInterface {
    pub url: Option<String>,
    pub transport: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde_with::skip_serializing_none]
pub struct AgentCardSignature {
    pub protected: Option<String>,
    pub signature: Option<String>,
    pub header: Option<BTreeMap<String, AgentJson>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde_with::skip_serializing_none]
pub struct AgentCard {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub protocol_version: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub name: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub description: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub url: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub version: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub capabilities: Option<Option<AgentCapabilities>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub default_input_modes: Option<Option<Vec<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub default_output_modes: Option<Option<Vec<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub skills: Option<Option<Vec<AgentSkill>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub preferred_transport: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub additional_interfaces: Option<Option<Vec<AgentInterface>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub icon_url: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub provider: Option<Option<AgentProvider>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub documentation_url: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub security_schemes: Option<Option<BTreeMap<String, SecurityScheme>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub security: Option<Option<SecurityRequirements>>,
    #[serde(rename = "supportsAuthenticatedExtendedCard")]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub supports_authenticated_card: Option<Option<bool>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub signatures: Option<Option<Vec<AgentCardSignature>>>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[serde_with::skip_serializing_none]
pub struct AugmentedAgentCard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Option<AgentCapabilities>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_input_modes: Option<Option<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_output_modes: Option<Option<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Option<Vec<AgentSkill>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_transport: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_interfaces: Option<Option<Vec<AgentInterface>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Option<AgentProvider>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_schemes: Option<Option<BTreeMap<String, SecurityScheme>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<Option<SecurityRequirements>>,
    #[serde(rename = "supportsAuthenticatedExtendedCard")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_authenticated_card: Option<Option<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signatures: Option<Option<Vec<AgentCardSignature>>>,
    #[serde(rename = "is_public")]
    pub is_public: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentObjectPermission {
    pub mcp_servers: Option<Vec<String>>,
    pub mcp_access_groups: Option<Vec<String>>,
    pub mcp_toolsets: Option<Vec<String>>,
    pub mcp_tool_permissions: Option<BTreeMap<String, Vec<String>>>,
    pub models: Option<Vec<String>>,
    pub agents: Option<Vec<String>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AgentKillSwitchAuth {
    Bearer(AgentBearerAuth),
    ApiKey(AgentApiKeyAuth),
    Basic(AgentBasicAuth),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBearerAuth {
    #[serde(rename = "type")]
    pub auth_type: String,
    #[serde(skip_serializing)]
    pub token: SecretString,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentApiKeyAuth {
    #[serde(rename = "type")]
    pub auth_type: String,
    #[serde(default = "default_api_key_header")]
    pub header_name: String,
    #[serde(skip_serializing)]
    pub api_key: SecretString,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBasicAuth {
    #[serde(rename = "type")]
    pub auth_type: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: SecretString,
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
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AgentConfig {
    pub identity: Option<EntraIdentityConfig>,
    pub enabled: Option<bool>,
    pub execution_mode: Option<ExecutionMode>,
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
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct PatchAgentRequest {
    pub identity: Option<EntraIdentityConfig>,
    pub enabled: Option<bool>,
    pub execution_mode: Option<ExecutionMode>,
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
#[serde(deny_unknown_fields)]
pub struct AgentCaller {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentKeySummary {
    pub token: String,
    pub key_alias: Option<String>,
    pub key_name: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResponse {
    pub identity: Option<AgentIdentityBinding>,
    #[serde(default)]
    pub identity_managed: AgentBoolean,
    #[serde(default = "default_enabled")]
    pub enabled: AgentBoolean,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub jwt_auth_configured: AgentBoolean,
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
