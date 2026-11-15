use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Debug, Formatter};

use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;

use crate::budget_permission_rows::Timestamp;
use crate::managed_resource_rows::mcp::{McpEnvVarScope, McpOAuth2Flow};
use crate::mcp_stored_contracts::pinned::MCPInfo;
use crate::mcp_stored_contracts::{JsonValue, MCPAuth, MCPCredentials, MCPTransport};

pub type FieldsSet = BTreeSet<String>;
pub type PreparedServerData = BTreeMap<String, JsonValue>;
pub type Maybe<T> = Option<Option<T>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerRequest {
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub alias: Option<String>,
    pub description: Option<String>,
    #[serde(default = "default_transport")]
    pub transport: MCPTransport,
    pub auth_type: Option<MCPAuth>,
    pub credentials: Option<MCPCredentials>,
    pub url: Option<String>,
    pub spec_path: Option<String>,
    pub mcp_info: Option<MCPInfo>,
    #[serde(default)]
    pub mcp_access_groups: Vec<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub tool_name_to_display_name: Option<BTreeMap<String, String>>,
    pub tool_name_to_description: Option<BTreeMap<String, String>>,
    pub extra_headers: Option<Vec<String>>,
    pub static_headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing)]
    pub env_vars: Option<Vec<EnvVarRequest>>,
    pub instructions: Option<String>,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
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
    #[serde(default)]
    pub allow_all_keys: RequestFlag,
    #[serde(default = "default_public_internet")]
    pub available_on_public_internet: RequestFlag,
    #[serde(default)]
    pub delegate_auth_to_upstream: RequestFlag,
    #[serde(default)]
    pub oauth_passthrough: RequestFlag,
    pub dcr_bridge: Option<bool>,
    #[serde(default)]
    pub per_server_oauth_discovery: RequestFlag,
    #[serde(default)]
    pub is_byok: RequestFlag,
    #[serde(default)]
    pub byok_description: Vec<String>,
    pub byok_api_key_help_url: Option<String>,
    pub source_url: Option<String>,
    pub timeout: Option<f64>,
    pub max_concurrent_requests: Option<i64>,
    pub approval_status: Option<String>,
    pub submitted_by: Option<String>,
    pub submitted_at: Option<Timestamp>,
}

fn default_transport() -> MCPTransport {
    MCPTransport::Sse
}

fn default_public_internet() -> RequestFlag {
    RequestFlag(true)
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
