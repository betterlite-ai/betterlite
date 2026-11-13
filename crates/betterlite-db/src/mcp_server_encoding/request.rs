use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Debug, Formatter};

use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;

use crate::budget_permission_rows::Timestamp;
use crate::managed_resource_rows::mcp::{McpEnvVarScope, McpOAuth2Flow};
use crate::mcp_server_encoding::credentials::CredentialBlob;
use crate::mcp_stored_contracts::pinned::MCPInfo;
use crate::mcp_stored_contracts::{JsonValue, MCPAuth, MCPTransport};

pub type FieldsSet = BTreeSet<String>;
pub type PreparedServerData = BTreeMap<String, JsonValue>;
pub type Maybe<T> = Option<Option<T>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestFlag(pub bool);

#[derive(Debug, Clone, Copy)]
pub enum PrepareMode<'fields> {
    Create,
    Update {
        model_fields: &'fields FieldsSet,
        original_fields: &'fields FieldsSet,
    },
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpServerRequest {
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub alias: Option<String>,
    pub description: Option<String>,
    pub transport: MCPTransport,
    pub auth_type: Option<MCPAuth>,
    pub credentials: Option<CredentialBlob>,
    pub url: Option<String>,
    pub spec_path: Option<String>,
    pub mcp_info: Option<MCPInfo>,
    pub mcp_access_groups: Vec<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub tool_name_to_display_name: Option<BTreeMap<String, String>>,
    pub tool_name_to_description: Option<BTreeMap<String, String>>,
    pub extra_headers: Option<Vec<String>>,
    pub static_headers: Option<BTreeMap<String, String>>,
    pub env_vars: Option<Vec<EnvVarRequest>>,
    pub instructions: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub issuer: Option<String>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub registration_url: Option<String>,
    pub oauth2_flow: Option<McpOAuth2Flow>,
    pub token_exchange_endpoint: Option<String>,
    pub audience: Option<String>,
    pub subject_token_type: Option<String>,
    pub token_exchange_profile: Option<String>,
    pub allow_all_keys: RequestFlag,
    pub available_on_public_internet: RequestFlag,
    pub delegate_auth_to_upstream: RequestFlag,
    pub oauth_passthrough: RequestFlag,
    pub dcr_bridge: Option<bool>,
    pub per_server_oauth_discovery: RequestFlag,
    pub is_byok: RequestFlag,
    pub byok_description: Vec<String>,
    pub byok_api_key_help_url: Option<String>,
    pub source_url: Option<String>,
    pub timeout: Option<f64>,
    pub max_concurrent_requests: Option<i64>,
    pub approval_status: Option<String>,
    pub submitted_by: Option<String>,
    pub submitted_at: Option<Timestamp>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvVarRequest {
    pub name: String,
    #[serde(default, with = "double_option")]
    pub value: Maybe<String>,
    #[serde(default, with = "double_option")]
    pub scope: Maybe<McpEnvVarScope>,
    #[serde(default, with = "double_option")]
    pub description: Maybe<String>,
}

impl Default for McpServerRequest {
    fn default() -> Self {
        Self {
            server_id: None,
            server_name: None,
            alias: None,
            description: None,
            transport: MCPTransport::Sse,
            auth_type: None,
            credentials: None,
            url: None,
            spec_path: None,
            mcp_info: None,
            mcp_access_groups: Vec::new(),
            allowed_tools: None,
            tool_name_to_display_name: None,
            tool_name_to_description: None,
            extra_headers: None,
            static_headers: None,
            env_vars: None,
            instructions: None,
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            issuer: None,
            authorization_url: None,
            token_url: None,
            registration_url: None,
            oauth2_flow: None,
            token_exchange_endpoint: None,
            audience: None,
            subject_token_type: None,
            token_exchange_profile: None,
            allow_all_keys: RequestFlag(false),
            available_on_public_internet: RequestFlag(true),
            delegate_auth_to_upstream: RequestFlag(false),
            oauth_passthrough: RequestFlag(false),
            dcr_bridge: None,
            per_server_oauth_discovery: RequestFlag(false),
            is_byok: RequestFlag(false),
            byok_description: Vec::new(),
            byok_api_key_help_url: None,
            source_url: None,
            timeout: None,
            max_concurrent_requests: None,
            approval_status: None,
            submitted_by: None,
            submitted_at: None,
        }
    }
}

impl Debug for McpServerRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpServerRequest").finish_non_exhaustive()
    }
}

impl Debug for EnvVarRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvVarRequest").finish_non_exhaustive()
    }
}
