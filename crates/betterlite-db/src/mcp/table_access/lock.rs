use blake2::Blake2b;
use blake2::digest::Digest as _;
use blake2::digest::consts::U8;
use deadpool_postgres::GenericClient as _;

use crate::postgres_store::errors::backend;

use super::McpIdentifierConflict;
use super::filter::{FindServers, ServerFilter, identifier_candidates};
use super::rows::server_id_of;
use super::store::{LockedTable, McpTableStore, db_transaction_manager};
use super::types::{LockedWrite, LockedWriteOutcome, McpServerAccess, McpTableError};

pub const ADVISORY_LOCK_SQL: &str = "SELECT pg_advisory_xact_lock($1::bigint)";
const MCP_IDENTIFIER_DOMAIN: &str = "mcp_identifier:";

// Mirrors LiteLLM's fn _identifier_where: Prisma equals with mode=insensitive over server_name and alias; approval_status NULL or not draft
#[must_use]
pub fn identifier_where(value: &str, exclude_server_id: Option<&str>) -> ServerFilter {
    ServerFilter::Identifiers {
        value: value.to_owned(),
        exclude_server_id: exclude_server_id.map(str::to_owned),
    }
}

// Mirrors LiteLLM's fn _mcp_identifier_lock_keys: blake2b over "mcp_identifier:{normalized}", digest_size=8 big-endian signed=True, keys from sorted(normalized)
#[must_use]
pub fn mcp_identifier_lock_keys(identifiers: &[Option<&str>]) -> Vec<i64> {
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
            filter: Some(identifier_where(value, exclude_server_id)),
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

// ast-grep-ignore: no-long-names - mirrors LiteLLM's fn _mcp_server_write_if_identifier_free
pub async fn mcp_server_write_if_identifier_free(
    store: &McpTableStore<'_>,
    request: &LockedWrite,
) -> Result<LockedWriteOutcome, McpTableError> {
    let mut manager = db_transaction_manager(store.client.writer).await?;
    let transaction = manager.begin().await?;
    // execute_raw's pg_advisory_xact_lock per identifier holds until this transaction ends
    let identifiers = [request.server_name.as_deref(), request.alias.as_deref()];
    for key in mcp_identifier_lock_keys(&identifiers) {
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
