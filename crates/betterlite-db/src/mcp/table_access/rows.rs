use std::collections::BTreeMap;

use crate::auxiliary_repository::repository::TableBinding;
use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::mcp::{McpServerTable, decode_server};
use crate::mcp_server_encoding::env_vars::decrypt_global_env_var_values;
use crate::mcp_stored_contracts::JsonValue;
use crate::stored_cipher::DbCipher;

use super::types::McpTableError;

pub type KeyValues = BTreeMap<&'static str, String>;

#[derive(Debug, Clone)]
pub struct RowFields(pub BTreeMap<&'static str, JsonValue>);

impl RowFields {
    #[must_use]
    pub fn texts(entries: &[(&'static str, &str)]) -> Self {
        let mut fields = BTreeMap::new();
        for (key, value) in entries {
            fields.insert(*key, JsonValue::String((*value).to_owned()));
        }
        Self(fields)
    }
}

#[derive(Debug, Clone)]
pub struct ServerUpdate {
    pub server_id: String,
    pub data: RowFields,
}

pub struct Table {
    pub name: &'static str,
    pub relation: &'static str,
    pub conflict_columns: &'static [&'static str],
    pub immutable_columns: &'static [&'static str],
}

pub const SERVER_TABLE: Table = Table {
    name: "litellm_mcpservertable",
    relation: "LiteLLM_MCPServerTable",
    conflict_columns: &["server_id"],
    immutable_columns: &["server_id"],
};

pub const CREDENTIAL_TABLE: Table = Table {
    name: "litellm_mcpusercredentials",
    relation: "LiteLLM_MCPUserCredentials",
    conflict_columns: &["user_id", "server_id"],
    immutable_columns: &["id", "user_id", "server_id"],
};

pub const OAUTH_CLIENT_TABLE: Table = Table {
    name: "litellm_mcpserveroauthclient",
    relation: "LiteLLM_MCPServerOAuthClient",
    conflict_columns: &["server_id"],
    immutable_columns: &["server_id"],
};

pub const ENV_VAR_TABLE: Table = Table {
    name: "litellm_mcpuserenvvars",
    relation: "LiteLLM_MCPUserEnvVars",
    conflict_columns: &["user_id", "server_id"],
    immutable_columns: &["id", "user_id", "server_id"],
};

pub struct McpUserEnvVarsRepository;

impl TableBinding for McpUserEnvVarsRepository {
    const NAME: &'static str = "litellm_mcpuserenvvars";
    const TABLE: &'static str = "LiteLLM_MCPUserEnvVars";
}

pub fn server_id_of(row: &JsonData) -> Result<String, McpTableError> {
    let JsonData::Object(fields) = row else {
        return Err(McpTableError::MissingServerId);
    };
    let Some(JsonData::String(server_id)) = fields.get("server_id") else {
        return Err(McpTableError::MissingServerId);
    };
    Ok(server_id.clone())
}

pub(crate) fn readable_server(
    row: &mut JsonData,
    cipher: &DbCipher,
) -> Result<McpServerTable, McpTableError> {
    let mut server = decode_server(row, cipher)?;
    if let Some(env_vars) = server.env_vars.as_mut() {
        decrypt_global_env_var_values(env_vars, cipher);
    }
    Ok(server)
}
