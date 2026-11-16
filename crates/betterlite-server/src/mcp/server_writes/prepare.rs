use std::collections::BTreeMap;

use betterlite_db::mcp::table_access::rows::RowFields;
use betterlite_db::mcp_server_encoding::prepare::prepare_mcp_server_data;
use betterlite_db::mcp_server_encoding::request::{
    McpServerRequest, PrepareMode, PreparedServerData,
};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::stored_cipher::DbCipher;

use super::ServerWriteError;

// Every column prepare_mcp_server_data and the audit fields can write, as LiteLLM_MCPServerTable names them.
const COLUMNS: [&str; 47] = [
    "alias",
    "allow_all_keys",
    "allowed_tools",
    "approval_status",
    "args",
    "audience",
    "auth_type",
    "authorization_url",
    "available_on_public_internet",
    "byok_api_key_help_url",
    "byok_description",
    "command",
    "created_by",
    "credentials",
    "dcr_bridge",
    "delegate_auth_to_upstream",
    "description",
    "env",
    "env_vars",
    "extra_headers",
    "instructions",
    "is_byok",
    "issuer",
    "max_concurrent_requests",
    "mcp_access_groups",
    "mcp_info",
    "oauth2_flow",
    "oauth_passthrough",
    "per_server_oauth_discovery",
    "registration_url",
    "server_id",
    "server_name",
    "source_url",
    "spec_path",
    "static_headers",
    "subject_token_type",
    "submitted_at",
    "submitted_by",
    "timeout",
    "token_exchange_endpoint",
    "token_exchange_profile",
    "token_url",
    "tool_name_to_description",
    "tool_name_to_display_name",
    "transport",
    "updated_by",
    "url",
];

// The create payload, with the audit fields LiteLLM adds before the insert.
pub fn create_fields(
    request: &McpServerRequest,
    touched_by: &str,
    cipher: &DbCipher,
) -> Result<RowFields, ServerWriteError> {
    let mut data = prepare_mcp_server_data(request, PrepareMode::Create, cipher)?;
    data.insert(
        "created_by".to_owned(),
        JsonValue::String(touched_by.to_owned()),
    );
    data.insert(
        "updated_by".to_owned(),
        JsonValue::String(touched_by.to_owned()),
    );
    row_fields(data)
}

// Prisma takes typed columns, while the prepared payload is keyed by the wire name.
pub fn row_fields(prepared: PreparedServerData) -> Result<RowFields, ServerWriteError> {
    let mut fields = BTreeMap::new();
    for (name, value) in prepared {
        let Some(column) = COLUMNS
            .iter()
            .copied()
            .find(|known| *known == name.as_str())
        else {
            return Err(ServerWriteError::UnknownColumn { name });
        };
        fields.insert(column, value);
    }
    Ok(RowFields(fields))
}
