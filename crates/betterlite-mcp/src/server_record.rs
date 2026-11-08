pub mod binding;
mod equality;
mod methods;
mod raw;
mod tests;

use std::collections::BTreeMap;
use std::fmt::{self, Debug, Display, Formatter};

use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_stored_contracts::{
    JsonValue, MCPAuth, MCPTokenEndpointAuthMethod, MCPTransport, MCPUpstreamProtocol,
    PinnedMCPTool,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::credential_contracts::errors::CredError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MCPOAuthMetadata {
    pub scopes: Option<Vec<String>>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub registration_url: Option<String>,
    pub discovered_issuer: Option<String>,
    #[serde(default)]
    pub from_origin_fallback: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "binding::RawIdentityBinding")]
pub struct MCPOAuthIdentityBinding {
    pub mode: binding::BindingMode,
    pub issuer: String,
    pub jwks_url: Option<String>,
    pub audiences: Vec<String>,
    pub principal_claim: String,
    pub caller_field: binding::CallerField,
    pub require_email_verified: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuth2Flow {
    ClientCredentials,
    AuthorizationCode,
}

#[derive(Clone, Deserialize, Serialize, derive_more::Display)]
#[serde(try_from = "raw::RawMCPServer")]
#[display(
    "MCPServer(server_id={server_id:?}, name={name:?}, transport={transport:?}, auth_type={auth_type:?})"
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "LiteLLM's MCPServer carries independent bool fields"
)]
pub struct MCPServer {
    pub server_id: String,
    pub name: String,
    pub alias: Option<String>,
    pub server_name: Option<String>,
    pub url: Option<String>,
    pub transport: MCPTransport,
    pub protocol_version: Option<MCPUpstreamProtocol>,
    pub spec_path: Option<String>,
    pub auth_type: Option<MCPAuth>,
    pub authentication_token: Option<StoredSecret>,
    pub instructions: Option<String>,
    pub mcp_info: Option<MCPInfo>,
    pub extra_headers: Option<Vec<String>>,
    pub allowed_tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,
    pub tool_name_to_display_name: Option<BTreeMap<String, String>>,
    pub tool_name_to_description: Option<BTreeMap<String, String>>,
    pub pinned_tools: Option<BTreeMap<String, PinnedMCPTool>>,
    pub allowed_params: Option<BTreeMap<String, Vec<String>>>,
    pub static_headers: Option<BTreeMap<String, String>>,
    pub env_vars: Option<Vec<JsonValue>>,
    pub client_id: Option<String>,
    pub client_secret: Option<StoredSecret>,
    pub issuer: Option<String>,
    pub issuer_is_anchored: bool,
    pub scopes: Option<Vec<String>>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub registration_url: Option<String>,
    pub configured_authorization_url: Option<String>,
    pub configured_token_url: Option<String>,
    pub configured_registration_url: Option<String>,
    pub configured_scopes: Option<Vec<String>>,
    pub token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
    pub upstream_resource: Option<String>,
    pub upstream_token_header: Option<String>,
    pub aws_access_key_id: Option<StoredSecret>,
    pub aws_secret_access_key: Option<StoredSecret>,
    pub aws_session_token: Option<StoredSecret>,
    pub aws_region_name: Option<String>,
    pub aws_service_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub token_exchange_endpoint: Option<String>,
    pub audience: Option<String>,
    pub subject_token_type: String,
    pub id_jag_resource_token_endpoint: Option<String>,
    pub id_jag_resource: Option<String>,
    pub client_private_key: Option<StoredSecret>,
    pub client_private_key_id: Option<String>,
    pub client_assertion_signing_alg: String,
    pub token_exchange_profile: String,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub access_groups: Option<Vec<String>>,
    pub allow_all_keys: bool,
    pub available_on_public_internet: bool,
    pub delegate_auth_to_upstream: bool,
    pub oauth_passthrough: bool,
    pub dcr_bridge: Option<bool>,
    pub per_server_oauth_discovery: bool,
    pub is_byok: bool,
    pub byok_description: Vec<String>,
    pub byok_api_key_help_url: Option<String>,
    pub source_url: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub oauth2_flow: Option<OAuth2Flow>,
    pub token_validation: Option<BTreeMap<String, JsonValue>>,
    pub oauth_identity_binding: Option<MCPOAuthIdentityBinding>,
    pub token_storage_ttl_seconds: Option<i64>,
    pub timeout: Option<f64>,
    pub max_concurrent_requests: Option<i64>,
    pub short_prefix: Option<String>,
    pub allow_sampling: bool,
    pub allow_elicitation: bool,
}

impl Debug for MCPServer {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

#[derive(Debug, Error)]
pub enum ServerRecordError {
    #[error(transparent)]
    Header(#[from] CredError),
    #[error("invalid protocol_version: {0}")]
    Protocol(#[from] serde_json::Error),
    #[error("{0}")]
    Binding(String),
}
