use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::{
    MCPAuth, MCPTokenEndpointAuthMethod, MCPTransport, MCPUpstreamProtocol,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer};

use crate::server_record::MCPOAuthIdentityBinding;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct McpServerConfig {
    pub server_id: Option<Json>,
    pub protocol_version: Option<MCPUpstreamProtocol>,
    pub alias: Option<String>,
    pub description: Option<String>,
    pub mcp_info: Option<IndexMap<String, Json>>,
    pub url: Option<String>,
    pub spec_path: Option<String>,
    pub transport: Option<MCPTransport>,
    pub auth_type: Option<MCPAuth>,
    pub authentication_token: Option<String>,
    pub auth_value: Option<String>,
    pub instructions: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub oauth2_flow: Option<String>,
    pub issuer: Option<String>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub registration_url: Option<String>,
    pub token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
    pub scopes: Option<ScopesConfig>,
    pub dcr_bridge: Option<Json>,
    #[serde(default, deserialize_with = "deserialize_present_json")]
    pub per_server_oauth_discovery: Option<Json>,
    pub extra_headers: Option<Vec<String>>,
    pub allowed_tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,
    pub allowed_params: Option<BTreeMap<String, Vec<String>>>,
    pub pinned_tools: Option<Json>,
    pub access_groups: Option<Vec<String>>,
    pub static_headers: Option<BTreeMap<String, String>>,
    pub env_vars: Option<Vec<IndexMap<String, Json>>>,
    pub allow_all_keys: Option<bool>,
    pub available_on_public_internet: Option<bool>,
    pub delegate_auth_to_upstream: Option<bool>,
    pub oauth_passthrough: Option<bool>,
    pub allow_sampling: Option<bool>,
    pub allow_elicitation: Option<bool>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    pub aws_region_name: Option<String>,
    pub aws_service_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub token_exchange_endpoint: Option<String>,
    pub token_exchange_profile: Option<String>,
    pub audience: Option<String>,
    pub subject_token_type: Option<String>,
    pub upstream_resource: Option<String>,
    pub upstream_token_header: Option<String>,
    pub id_jag_resource_token_endpoint: Option<String>,
    pub id_jag_resource: Option<String>,
    pub client_private_key: Option<String>,
    pub client_private_key_id: Option<String>,
    pub client_assertion_signing_alg: Option<String>,
    pub timeout: Option<f64>,
    pub max_concurrent_requests: Option<i64>,
    pub token_validation: Option<Json>,
    pub oauth_identity_binding: Option<MCPOAuthIdentityBinding>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ScopesConfig {
    Text(String),
    List(Vec<String>),
}

// An explicit null must stay distinguishable from a missing key, so this bypasses Option's null handling
fn deserialize_present_json<'de, D>(deserializer: D) -> Result<Option<Json>, D::Error>
where
    D: Deserializer<'de>,
{
    Json::deserialize(deserializer).map(Some)
}
