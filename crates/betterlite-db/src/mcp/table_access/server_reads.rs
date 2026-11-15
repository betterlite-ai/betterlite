use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::mcp::McpServerTable;
use crate::mcp_stored_contracts::JsonValue;
use crate::stored_cipher::DbCipher;

use super::filter::{FindServers, ServerFilter};
use super::rows::{RowFields, readable_server};
use super::types::{McpServerAccess, McpTableError};

#[must_use]
pub fn _identifier_field(fields: &RowFields, field: &str) -> Option<String> {
    let Some(JsonValue::String(value)) = fields.0.get(field) else {
        return None;
    };
    Some(value.clone())
}

pub async fn _db_find_mcp_server_rows<Access>(
    table: &Access,
    filter: Option<ServerFilter>,
) -> Result<Vec<JsonData>, McpTableError>
where
    Access: McpServerAccess,
{
    table
        .find_servers(&FindServers {
            filter,
            descending: false,
            take: None,
        })
        .await
}

pub async fn get_mcp_server<Access>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
) -> Result<Option<McpServerTable>, McpTableError>
where
    Access: McpServerAccess,
{
    let Some(mut row) = super::_db_find_mcp_server_row(table, server_id).await? else {
        return Ok(None);
    };
    readable_server(&mut row, cipher).map(Some)
}

// the IN predicate returns rows in table order, not in the caller's order
pub async fn get_mcp_servers<Access>(
    table: &Access,
    cipher: &DbCipher,
    server_ids: &[String],
) -> Result<Vec<McpServerTable>, McpTableError>
where
    Access: McpServerAccess,
{
    let filter = ServerFilter::ServerIds(server_ids.to_vec());
    let rows = _db_find_mcp_server_rows(table, Some(filter)).await?;
    super::_readable_mcp_servers(rows, cipher)
}
