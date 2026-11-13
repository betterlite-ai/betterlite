use std::collections::BTreeMap;

use betterlite_secrets::secret_codec::secret_map::decode_secret_map;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::JsonData;
use crate::mcp_stored_contracts::JsonValue as StoredJson;
use crate::mcp_stored_contracts::pinned::{MCPInfo, parse_pinned_tools};
use crate::mcp_stored_contracts::{MCPAuth, MCPCredentials, MCPTransport, PinnedMCPTool};
use crate::stored_cipher::DbCipher;

use super::ManagedRowsDecodeError;
use super::wire::{PublicInternet, RowFlag, RowTimestamp, SecretMap, SecretMapField};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpEnvVarScope {
    #[default]
    #[serde(rename = "global")]
    Global,
    #[serde(rename = "user")]
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpEnvVar {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub scope: McpEnvVarScope,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpStatus {
    Healthy,
    Reachable,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpOAuth2Flow {
    #[serde(rename = "client_credentials")]
    ClientCredentials,
    #[serde(rename = "authorization_code")]
    AuthorizationCode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerTable {
    pub server_id: String,
    #[serde(default)]
    pub is_config: RowFlag,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub spec_path: Option<String>,
    pub transport: MCPTransport,
    #[serde(default)]
    pub auth_type: Option<MCPAuth>,
    #[serde(default)]
    pub credentials: Option<MCPCredentials>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub created_at: Option<RowTimestamp>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_at: Option<RowTimestamp>,
    #[serde(default)]
    pub updated_by: Option<String>,
    #[serde(default)]
    pub teams: Vec<BTreeMap<String, Option<String>>>,
    #[serde(default)]
    pub mcp_access_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub tool_name_to_display_name: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub tool_name_to_description: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub pinned_tools: Option<BTreeMap<String, PinnedMCPTool>>,
    #[serde(default)]
    pub extra_headers: Vec<String>,
    #[serde(default)]
    pub mcp_info: Option<MCPInfo>,
    #[serde(default)]
    pub static_headers: Option<SecretMapField>,
    #[serde(default)]
    pub env_vars: Option<Vec<McpEnvVar>>,
    #[serde(default = "default_status")]
    pub status: Option<McpStatus>,
    #[serde(default)]
    pub last_health_check: Option<RowTimestamp>,
    #[serde(default)]
    pub health_check_error: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: SecretMapField,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub authorization_url: Option<String>,
    #[serde(default)]
    pub token_url: Option<String>,
    #[serde(default)]
    pub registration_url: Option<String>,
    #[serde(default)]
    pub oauth2_flow: Option<McpOAuth2Flow>,
    #[serde(default)]
    pub token_exchange_endpoint: Option<String>,
    #[serde(default)]
    pub audience: Option<String>,
    #[serde(default)]
    pub subject_token_type: Option<String>,
    #[serde(default)]
    pub token_exchange_profile: Option<String>,
    #[serde(default)]
    pub allow_all_keys: RowFlag,
    #[serde(default)]
    pub available_on_public_internet: PublicInternet,
    #[serde(default)]
    pub delegate_auth_to_upstream: RowFlag,
    #[serde(default)]
    pub oauth_passthrough: RowFlag,
    #[serde(default)]
    pub dcr_bridge: Option<bool>,
    #[serde(default)]
    pub per_server_oauth_discovery: RowFlag,
    #[serde(default)]
    pub is_byok: RowFlag,
    #[serde(default)]
    pub byok_description: Vec<String>,
    #[serde(default)]
    pub byok_api_key_help_url: Option<String>,
    #[serde(default)]
    pub has_user_credential: Option<bool>,
    #[serde(default)]
    pub connected_app_reachable: Option<bool>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub timeout: Option<Decimal>,
    #[serde(default)]
    pub max_concurrent_requests: Option<i64>,
    #[serde(default = "default_approval_status")]
    pub approval_status: Option<String>,
    #[serde(default)]
    pub submitted_by: Option<String>,
    #[serde(default)]
    pub submitted_at: Option<RowTimestamp>,
    #[serde(default)]
    pub reviewed_at: Option<RowTimestamp>,
    #[serde(default)]
    pub review_notes: Option<String>,
}

#[must_use]
pub const fn default_status() -> Option<McpStatus> {
    Some(McpStatus::Unknown)
}

#[must_use]
pub fn default_approval_status() -> Option<String> {
    Some("active".to_owned())
}

pub(crate) fn decode_server(
    row: &mut JsonData,
    cipher: &DbCipher,
) -> Result<McpServerTable, ManagedRowsDecodeError> {
    let (static_headers, env, pinned_tools, mcp_info) = {
        let fields = super::object_fields(row)?;
        let static_headers = take_secret_map(fields, "static_headers", cipher)?;
        let env = take_secret_map(fields, "env", cipher)?.unwrap_or_default();
        let pinned_tools = take_pinned_tools(fields)?;
        let mcp_info = take_mcp_info(fields)?;
        (static_headers, env, pinned_tools, mcp_info)
    };
    let mut server: McpServerTable = super::decode_row(row)?;
    server.static_headers = static_headers.map(SecretMapField);
    server.env = SecretMapField(env);
    server.pinned_tools = pinned_tools;
    server.mcp_info = mcp_info;
    Ok(server)
}

fn take_secret_map(
    fields: &mut BTreeMap<String, JsonData>,
    field: &'static str,
    cipher: &DbCipher,
) -> Result<Option<SecretMap>, ManagedRowsDecodeError> {
    let Some(value) = fields.remove(field) else {
        return Ok(None);
    };
    if matches!(value, JsonData::Null) {
        return Ok(None);
    }
    let encoded = serde_json::to_string(&value).map_err(ManagedRowsDecodeError::Json)?;
    decode_secret_map(cipher, Some(&encoded), field).map_err(ManagedRowsDecodeError::SecretMap)
}

fn take_pinned_tools(
    fields: &mut BTreeMap<String, JsonData>,
) -> Result<Option<BTreeMap<String, PinnedMCPTool>>, ManagedRowsDecodeError> {
    let Some(value) = fields.remove("pinned_tools") else {
        return Ok(None);
    };
    let encoded = serde_json::to_string(&value).map_err(ManagedRowsDecodeError::Json)?;
    let stored =
        serde_json::from_str::<StoredJson>(&encoded).map_err(ManagedRowsDecodeError::Json)?;
    parse_pinned_tools(stored).map_err(ManagedRowsDecodeError::PinnedTools)
}

fn take_mcp_info(
    fields: &mut BTreeMap<String, JsonData>,
) -> Result<Option<MCPInfo>, ManagedRowsDecodeError> {
    let Some(value) = fields.remove("mcp_info") else {
        return Ok(None);
    };
    if matches!(value, JsonData::Null) {
        return Ok(None);
    }
    let encoded = serde_json::to_string(&value).map_err(ManagedRowsDecodeError::Json)?;
    serde_json::from_str(&encoded)
        .map(Some)
        .map_err(ManagedRowsDecodeError::Json)
}
