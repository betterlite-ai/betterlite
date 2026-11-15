use std::collections::BTreeMap;

use blake2::Blake2b;
use blake2::digest::Digest as _;
use blake2::digest::consts::U8;
use chrono::{SecondsFormat, Utc};
use deadpool_postgres::GenericClient as _;

use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::ManagedRowsDecodeError;
use crate::managed_resource_rows::mcp::McpServerTable;
use crate::mcp_stored_contracts::PinnedMCPTool;
use crate::postgres_store::errors::backend;
use crate::stored_cipher::DbCipher;

pub mod actions;
pub mod filter;
pub mod rows;
pub mod server_reads;
pub mod sql;
pub mod store;
pub mod types;
pub mod user_tables;

mod mcp_table_access;

use filter::{FindServers, ServerFilter, identifier_candidates};
use rows::{RowFields, ServerUpdate, server_id_of};
use server_reads::_db_find_mcp_server_rows;
use store::{_db_transaction_manager, LockedTable, McpTableStore};
use types::{
    ACTIVE, IdentifierField, LockedWrite, LockedWriteOutcome, MCPSubmissionsSummary,
    McpServerAccess, McpTableError, PENDING_REVIEW, REJECTED,
};

pub const ADVISORY_LOCK_SQL: &str = "SELECT pg_advisory_xact_lock($1::bigint)";
const MCP_IDENTIFIER_DOMAIN: &str = "mcp_identifier:";

// field is "server_name" or "alias": the stored identifier the incoming value collides with
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpIdentifierConflict {
    pub field: IdentifierField,
    pub value: String,
    pub server_id: String,
}

// Prisma equals with mode=insensitive over server_name and alias; approval_status NULL or not draft
#[must_use]
pub fn _identifier_where(value: &str, exclude_server_id: Option<&str>) -> ServerFilter {
    ServerFilter::Identifiers {
        value: value.to_owned(),
        exclude_server_id: exclude_server_id.map(str::to_owned),
    }
}

// blake2b over "mcp_identifier:{normalized}", digest_size=8 big-endian signed=True, keys from sorted(normalized)
#[must_use]
pub fn _mcp_identifier_lock_keys(identifiers: &[Option<&str>]) -> Vec<i64> {
    let mut normalized = Vec::with_capacity(identifiers.len());
    for identifier in identifiers.iter().flatten() {
        if !identifier.is_empty() {
            normalized.push(identifier.to_lowercase());
        }
    }
    normalized.sort();
    normalized.dedup();
    let mut keys = Vec::with_capacity(normalized.len());
    for identifier in &normalized {
        let mut hasher = Blake2b::<U8>::new();
        hasher.update(MCP_IDENTIFIER_DOMAIN.as_bytes());
        hasher.update(identifier.as_bytes());
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&hasher.finalize());
        keys.push(i64::from_be_bytes(bytes));
    }
    keys
}

pub async fn find_identifier_conflict<Access>(
    table: &Access,
    server_name: Option<&str>,
    alias: Option<&str>,
    exclude_server_id: Option<&str>,
) -> Result<Option<McpIdentifierConflict>, McpTableError>
where
    Access: McpServerAccess,
{
    // alias is checked before server_name so the reported field is deterministic
    for (field, value) in identifier_candidates(server_name, alias) {
        let request = FindServers {
            filter: Some(_identifier_where(value, exclude_server_id)),
            descending: false,
            take: Some(1),
        };
        if let Some(row) = table.find_servers(&request).await?.first() {
            return Ok(Some(McpIdentifierConflict {
                field,
                value: value.to_owned(),
                server_id: server_id_of(row)?,
            }));
        }
    }
    Ok(None)
}

// ast-grep-ignore: no-long-names
pub async fn _mcp_server_write_if_identifier_free(
    store: &McpTableStore<'_>,
    request: &LockedWrite,
) -> Result<LockedWriteOutcome, McpTableError> {
    let mut manager = _db_transaction_manager(store.client.writer).await?;
    let transaction = manager.begin().await?;
    // execute_raw's pg_advisory_xact_lock per identifier holds until this transaction ends
    let identifiers = [request.server_name.as_deref(), request.alias.as_deref()];
    for key in _mcp_identifier_lock_keys(&identifiers) {
        transaction
            .execute(ADVISORY_LOCK_SQL, &[&key])
            .await
            .map_err(|source| McpTableError::Database(backend(source)))?;
    }
    let locked = LockedTable {
        transaction: &transaction,
    };
    let conflict = find_identifier_conflict(
        &locked,
        request.server_name.as_deref(),
        request.alias.as_deref(),
        request.exclude_server_id.as_deref(),
    )
    .await?;
    if let Some(collision) = conflict {
        return Ok(LockedWriteOutcome::Conflict(collision));
    }
    let written = locked.write(&request.write).await?;
    transaction
        .commit()
        .await
        .map_err(|source| McpTableError::Database(backend(source)))?;
    Ok(LockedWriteOutcome::Written(written))
}

// Prisma's find_unique on the server_id primary key
pub async fn _db_find_mcp_server_row<Access>(
    table: &Access,
    server_id: &str,
) -> Result<Option<JsonData>, McpTableError>
where
    Access: McpServerAccess,
{
    let unique = FindServers {
        filter: Some(ServerFilter::ServerId(server_id.to_owned())),
        descending: false,
        take: Some(1),
    };
    let mut found = table.find_servers(&unique).await?;
    Ok(found.pop())
}

// missing row raises ValueError("MCP server not found, passed server_id={server_id}")
pub async fn _db_update_mcp_server_row<Access>(
    table: &Access,
    server_id: &str,
    data: RowFields,
) -> Result<JsonData, McpTableError>
where
    Access: McpServerAccess,
{
    let update = ServerUpdate {
        server_id: server_id.to_owned(),
        data,
    };
    table
        .update_server(&update)
        .await?
        .ok_or_else(|| McpTableError::ServerNotFound {
            server_id: server_id.to_owned(),
        })
}

pub fn _readable_mcp_servers(
    rows: Vec<JsonData>,
    cipher: &DbCipher,
) -> Result<Vec<McpServerTable>, McpTableError> {
    let mut servers = Vec::with_capacity(rows.len());
    for mut row in rows {
        match rows::readable_server(&mut row, cipher) {
            Ok(server) => servers.push(server),
            Err(McpTableError::Decode(ManagedRowsDecodeError::SecretMap(error))) => {
                let server_id = server_id_of(&row).unwrap_or_default();
                tracing::warn!(%error, "Skipping MCP server {server_id}: cannot decrypt secret map");
            }
            Err(error) => return Err(error),
        }
    }
    Ok(servers)
}

// line up with _db_find_mcp_server_rows: approval_status None means every row where it is NULL or not draft
pub async fn get_all_mcp_servers<Access>(
    table: &Access,
    cipher: &DbCipher,
    approval_status: Option<&str>,
) -> Result<Vec<McpServerTable>, McpTableError>
where
    Access: McpServerAccess,
{
    let filter = approval_status.map_or(ServerFilter::NotDraft, |status| {
        ServerFilter::ApprovalStatus(status.to_owned())
    });
    let rows = _db_find_mcp_server_rows(table, Some(filter)).await?;
    _readable_mcp_servers(rows, cipher)
}

// reviewed_at is the current UTC time and updated_by the reviewer on both review outcomes
async fn review_server<Access>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
    touched_by: &str,
    outcome: &[(&'static str, &str)],
) -> Result<McpServerTable, McpTableError>
where
    Access: McpServerAccess,
{
    let reviewed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut entries = vec![
        ("reviewed_at", reviewed_at.as_str()),
        ("updated_by", touched_by),
    ];
    entries.extend_from_slice(outcome);
    let mut row = _db_update_mcp_server_row(table, server_id, RowFields::texts(&entries)).await?;
    rows::readable_server(&mut row, cipher)
}

pub async fn approve_mcp_server<Access>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
    touched_by: &str,
) -> Result<McpServerTable, McpTableError>
where
    Access: McpServerAccess,
{
    review_server(
        table,
        cipher,
        server_id,
        touched_by,
        &[("approval_status", ACTIVE)],
    )
    .await
}

// approval_status rejected; review_notes is written only when given, so a reject without notes keeps the stored ones
pub async fn reject_mcp_server<Access>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
    touched_by: &str,
    review_notes: Option<&str>,
) -> Result<McpServerTable, McpTableError>
where
    Access: McpServerAccess,
{
    let mut outcome = vec![("approval_status", REJECTED)];
    if let Some(notes) = review_notes {
        outcome.push(("review_notes", notes));
    }
    review_server(table, cipher, server_id, touched_by, &outcome).await
}

// model_dump on each tool then safe_dumps over the mapping, which the transparent PinnedMCPTool reproduces
pub async fn set_mcp_server_pinned_tools<Access>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
    pinned_tools: Option<&BTreeMap<String, PinnedMCPTool>>,
    touched_by: &str,
) -> Result<Option<McpServerTable>, McpTableError>
where
    Access: McpServerAccess,
{
    if _db_find_mcp_server_row(table, server_id).await?.is_none() {
        return Ok(None);
    }
    let snapshot = serde_json::to_string(&pinned_tools.cloned().unwrap_or_default())
        .map_err(McpTableError::Json)?;
    let data = RowFields::texts(&[
        ("pinned_tools", snapshot.as_str()),
        ("updated_by", touched_by),
    ]);
    let mut row = _db_update_mcp_server_row(table, server_id, data).await?;
    rows::readable_server(&mut row, cipher).map(Some)
}

// ast-grep-ignore: no-long-names
pub async fn get_active_submitted_mcp_server_ids_for_user<Access>(
    table: &Access,
    user_id: &str,
) -> Result<Vec<String>, McpTableError>
where
    Access: McpServerAccess,
{
    if user_id.is_empty() {
        return Ok(Vec::new());
    }
    // MCPApprovalStatus active: creator visibility keeps only rows the user submitted that are approved
    let filter = ServerFilter::SubmittedByActive {
        user_id: user_id.to_owned(),
    };
    let rows = _db_find_mcp_server_rows(table, Some(filter)).await?;
    let mut server_ids = Vec::with_capacity(rows.len());
    for row in &rows {
        server_ids.push(server_id_of(row)?);
    }
    Ok(server_ids)
}

// counts across pending_review, active and rejected submissions; total is the number of items
pub async fn get_mcp_submissions<Access>(
    table: &Access,
    cipher: &DbCipher,
) -> Result<MCPSubmissionsSummary, McpTableError>
where
    Access: McpServerAccess,
{
    let request = FindServers {
        filter: Some(ServerFilter::Submitted),
        descending: true,
        take: Some(500),
    };
    let items = _readable_mcp_servers(table.find_servers(&request).await?, cipher)?;
    let pending_review = items
        .iter()
        .filter(|server| server.approval_status.as_deref() == Some(PENDING_REVIEW))
        .count();
    let active = items
        .iter()
        .filter(|server| server.approval_status.as_deref() == Some(ACTIVE))
        .count();
    let rejected = items
        .iter()
        .filter(|server| server.approval_status.as_deref() == Some(REJECTED))
        .count();
    Ok(MCPSubmissionsSummary {
        total: items.len(),
        pending_review,
        active,
        rejected,
        items,
    })
}
