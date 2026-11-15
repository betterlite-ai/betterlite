use std::collections::BTreeMap;

use blake2::Blake2b;
use blake2::digest::Digest as _;
use blake2::digest::consts::U8;
use chrono::{SecondsFormat, Utc};

use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::ManagedRowsDecodeError;
use crate::managed_resource_rows::mcp::McpServerTable;
use crate::mcp_stored_contracts::PinnedMCPTool;
use crate::stored_cipher::DbCipher;

pub mod actions;
pub mod filter;
pub mod rows;
pub mod server_reads;
pub mod sql;
pub mod store;
pub mod types;
pub mod user_tables;

#[cfg(test)]
mod mcp_table_access;

use filter::{FindServers, ServerFilter, identifier_candidates};
use rows::{RowFields, ServerUpdate, server_id_of};
use server_reads::{_db_find_mcp_server_row, find_readable};
use store::McpTableStore;
use types::{
    ACTIVE, IdentifierField, LockedWrite, LockedWriteOutcome, MCPSubmissionsSummary,
    McpServerAccess, McpTableError, PENDING_REVIEW, REJECTED, ReviewUpdate,
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
        keys.push(lock_key(identifier));
    }
    keys
}

fn lock_key(normalized: &str) -> i64 {
    let mut hasher = Blake2b::<U8>::new();
    hasher.update(MCP_IDENTIFIER_DOMAIN.as_bytes());
    hasher.update(normalized.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&hasher.finalize());
    i64::from_be_bytes(bytes)
}

pub async fn find_identifier_conflict<Access: McpServerAccess>(
    table: &Access,
    server_name: Option<&str>,
    alias: Option<&str>,
    exclude_server_id: Option<&str>,
) -> Result<Option<McpIdentifierConflict>, McpTableError> {
    // alias is checked before server_name so the reported field is deterministic
    for (field, value) in identifier_candidates(server_name, alias) {
        let filter = Some(_identifier_where(value, exclude_server_id));
        let request = FindServers {
            filter,
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

// ast-grep-ignore: no-long-names, no-one-line-functions
pub async fn _mcp_server_write_if_identifier_free(
    store: &McpTableStore<'_>,
    request: &LockedWrite,
) -> Result<LockedWriteOutcome, McpTableError> {
    store.write_if_identifier_free(request).await
}

// missing row raises ValueError("MCP server not found, passed server_id={server_id}")
pub async fn _db_update_mcp_server_row<Access: McpServerAccess>(
    table: &Access,
    server_id: &str,
    data: RowFields,
) -> Result<JsonData, McpTableError> {
    let update = ServerUpdate {
        server_id: server_id.to_owned(),
        data,
    };
    let Some(row) = table.update_server(&update).await? else {
        return Err(McpTableError::ServerNotFound {
            server_id: server_id.to_owned(),
        });
    };
    Ok(row)
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
pub async fn get_all_mcp_servers<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
    approval_status: Option<&str>,
) -> Result<Vec<McpServerTable>, McpTableError> {
    let filter = approval_status.map_or(ServerFilter::NotDraft, |status| {
        ServerFilter::ApprovalStatus(status.to_owned())
    });
    find_readable(table, cipher, filter, None).await
}

// approval_status active after approve; rejected with review_notes (when given) after reject
async fn review_server<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
    touched_by: &str,
    review: ReviewUpdate<'_>,
) -> Result<McpServerTable, McpTableError> {
    let reviewed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut entries = vec![
        ("approval_status", review.status),
        ("reviewed_at", reviewed_at.as_str()),
        ("updated_by", touched_by),
    ];
    if let Some(notes) = review.notes {
        entries.push(("review_notes", notes));
    }
    let mut row = _db_update_mcp_server_row(table, server_id, RowFields::texts(entries)).await?;
    rows::readable_server(&mut row, cipher)
}

pub async fn approve_mcp_server<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
    touched_by: &str,
) -> Result<McpServerTable, McpTableError> {
    let review = ReviewUpdate {
        status: ACTIVE,
        notes: None,
    };
    review_server(table, cipher, server_id, touched_by, review).await
}

pub async fn reject_mcp_server<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
    touched_by: &str,
    review_notes: Option<&str>,
) -> Result<McpServerTable, McpTableError> {
    let review = ReviewUpdate {
        status: REJECTED,
        notes: review_notes,
    };
    review_server(table, cipher, server_id, touched_by, review).await
}

// model_dump on each tool then safe_dumps over the mapping, which the transparent PinnedMCPTool reproduces
pub async fn set_mcp_server_pinned_tools<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
    server_id: &str,
    pinned_tools: Option<&BTreeMap<String, PinnedMCPTool>>,
    touched_by: &str,
) -> Result<Option<McpServerTable>, McpTableError> {
    if _db_find_mcp_server_row(table, server_id).await?.is_none() {
        return Ok(None);
    }
    let snapshot = rows::pinned_snapshot(pinned_tools)?;
    let data = RowFields::texts([
        ("pinned_tools", snapshot.as_str()),
        ("updated_by", touched_by),
    ]);
    let mut row = _db_update_mcp_server_row(table, server_id, data).await?;
    rows::readable_server(&mut row, cipher).map(Some)
}

// ast-grep-ignore: no-long-names
pub async fn get_active_submitted_mcp_server_ids_for_user<Access: McpServerAccess>(
    table: &Access,
    user_id: &str,
) -> Result<Vec<String>, McpTableError> {
    if user_id.is_empty() {
        return Ok(Vec::new());
    }
    // MCPApprovalStatus active: creator visibility keeps only rows the user submitted that are approved
    let filter = ServerFilter::SubmittedByActive {
        user_id: user_id.to_owned(),
    };
    let rows = server_reads::_db_find_mcp_server_rows(table, Some(filter)).await?;
    let mut server_ids = Vec::with_capacity(rows.len());
    for row in &rows {
        server_ids.push(server_id_of(row)?);
    }
    Ok(server_ids)
}

// counts across pending_review, active and rejected submissions; total is the number of items
pub async fn get_mcp_submissions<Access: McpServerAccess>(
    table: &Access,
    cipher: &DbCipher,
) -> Result<MCPSubmissionsSummary, McpTableError> {
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
