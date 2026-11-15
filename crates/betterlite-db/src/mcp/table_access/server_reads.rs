use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::mcp::McpServerTable;
use crate::mcp_stored_contracts::JsonValue;
use crate::stored_cipher::DbCipher;

use super::db_find_mcp_server_rows;
use super::filter::ServerFilter;
use super::rows::{RowFields, readable_server};
use super::types::{McpServerAccess, McpTableError};

// Mirrors LiteLLM's fn _identifier_field: the mapped value only when it is a string
#[must_use]
pub fn identifier_field(fields: &RowFields, field: &str) -> Option<String> {
    let Some(JsonValue::String(value)) = fields.0.get(field) else {
        return None;
    };
    Some(value.clone())
}

// Mirrors LiteLLM's fn _db_find_mcp_server_row, Prisma's find_unique on the server_id primary key
pub async fn db_find_mcp_server_row<Access>(
    table: &Access,
    server_id: &str,
) -> Result<Option<JsonData>, McpTableError>
where
    Access: McpServerAccess,
{
    let key = ServerFilter::ServerId(server_id.to_owned());
    let mut found = db_find_mcp_server_rows(table, Some(key)).await?;
    Ok(found.pop())
}

pub async fn get_mcp_server<Access>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
) -> Result<Option<McpServerTable>, McpTableError>
where
    Access: McpServerAccess,
{
    let Some(mut row) = db_find_mcp_server_row(table, server_id).await? else {
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
    let rows = db_find_mcp_server_rows(table, Some(filter)).await?;
    super::readable_mcp_servers(rows, cipher)
}
