use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::readable_mcp_servers;
use betterlite_db::mcp::table_access::rows::server_id_of;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_db::stored_cipher::DbCipher;
use serde::de::DeserializeOwned;

use super::ServerWriteError;

// LiteLLM writes these through safe_dumps, so a row read back carries their JSON text.
const JSON_TEXT_COLUMNS: [&str; 5] = [
    "credentials",
    "env_vars",
    "mcp_info",
    "tool_name_to_display_name",
    "tool_name_to_description",
];

// Read a written row the way LiteLLM's model_validate would, with the text columns parsed first.
pub fn decode_server_row(
    mut row: JsonData,
    cipher: &DbCipher,
) -> Result<McpServerTable, ServerWriteError> {
    let server_id = server_id_of(&row)?;
    if let JsonData::Object(fields) = &mut row {
        parse_text_columns(fields)?;
    }
    let mut servers = readable_mcp_servers(vec![row], cipher)?;
    servers
        .pop()
        .ok_or(ServerWriteError::UnreadableRow { server_id })
}

fn parse_text_columns(fields: &mut BTreeMap<String, JsonData>) -> Result<(), ServerWriteError> {
    for column in JSON_TEXT_COLUMNS {
        // Only a JSON string is text to parse; an object already has the shape the row wants.
        let Some(JsonData::String(text)) = fields.get(column) else {
            continue;
        };
        let parsed: JsonData = serde_json::from_str(text).map_err(ServerWriteError::Json)?;
        fields.insert(column.to_owned(), parsed);
    }
    Ok(())
}

// Read the few stored columns a write path has to see before it can decide what to write.
pub fn row_as<T>(row: &JsonData) -> Result<T, McpTableError>
where
    T: DeserializeOwned,
{
    serde_json::to_value(row)
        .and_then(serde_json::from_value)
        .map_err(McpTableError::Json)
}
