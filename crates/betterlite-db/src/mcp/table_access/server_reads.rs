use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::mcp::McpServerTable;
use crate::mcp_stored_contracts::JsonValue;
use crate::stored_cipher::DbCipher;

use super::filter::{FindServers, ServerFilter};
use super::rows::{RowFields, readable_server};
use super::types::{McpServerAccess, McpTableError};

#[must_use]
pub fn _identifier_field(fields: &RowFields, field: &str) -> Option<String> {
    match fields.0.get(field) {
        Some(JsonValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

pub(crate) async fn find_readable<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
    filter: ServerFilter,
    take: Option<u32>,
) -> Result<Vec<McpServerTable>, McpTableError> {
    let request = FindServers {
        filter: Some(filter),
        descending: false,
        take,
    };
    super::_readable_mcp_servers(table.find_servers(&request).await?, cipher)
}

pub async fn _db_find_mcp_server_rows<Access: McpServerAccess>(
    table: &Access,
    filter: Option<ServerFilter>,
) -> Result<Vec<JsonData>, McpTableError> {
    let request = FindServers {
        filter,
        descending: false,
        take: None,
    };
    table.find_servers(&request).await
}

pub async fn _db_find_mcp_server_row<Access: McpServerAccess>(
    table: &Access,
    server_id: &str,
) -> Result<Option<JsonData>, McpTableError> {
    let filter = ServerFilter::ServerId(server_id.to_owned());
    let rows = _db_find_mcp_server_rows(table, Some(filter)).await?;
    Ok(rows.into_iter().next())
}

pub async fn get_mcp_server<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
) -> Result<Option<McpServerTable>, McpTableError> {
    let filter = Some(ServerFilter::ServerId(server_id.to_owned()));
    let request = FindServers {
        filter,
        descending: false,
        take: Some(1),
    };
    let Some(mut row) = table.find_servers(&request).await?.pop() else {
        return Ok(None);
    };
    readable_server(&mut row, cipher).map(Some)
}

pub async fn get_mcp_servers<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
    server_ids: &[String],
) -> Result<Vec<McpServerTable>, McpTableError> {
    // the IN predicate returns rows in table order, not in the caller's order
    let filter = ServerFilter::ServerIds(server_ids.to_vec());
    find_readable(table, cipher, filter, None).await
}
