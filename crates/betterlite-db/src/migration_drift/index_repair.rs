use regex::Regex;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio_postgres::{Client as PostgresClient, Error as PostgresError, GenericClient};

use crate::database_tls::translation::strip_prisma_query_params;
use crate::migration_ledger::connection::connect;
use crate::migration_ledger::environment::{DEFAULT_SCHEMA, prisma_schema_param};
use crate::migration_ledger::lock::{HeldLockError, held_migration_lock};

use super::MigrationDriftCheck;

// int.from_bytes(b"litellm", "big") keeps one session-wide gate shared with LiteLLM's repair run
pub const INDEX_REPAIR_LOCK_KEY: i64 = i64::from_be_bytes(*b"\0litellm");
const INVALID_INDEXES_SQL: &str = "SELECT n.nspname, c.relname, pg_size_pretty(pg_table_size(t.oid)) \
    FROM pg_index i \
    JOIN pg_class c ON c.oid = i.indexrelid \
    JOIN pg_class t ON t.oid = i.indrelid \
    JOIN pg_namespace n ON n.oid = t.relnamespace \
    WHERE NOT i.indisvalid \
      AND c.relkind = 'i' \
      AND n.nspname = $1 \
      AND t.relname LIKE $2 \
      AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = i.indexrelid) \
    ORDER BY c.relname";
const LITELLM_TABLE_PATTERN: &str = r"LiteLLM\_%";
const TRANSIENT_INDEX_SUFFIX: &str = r"_cc(?:new|old)\d*$";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidIndex {
    pub schema: String,
    pub name: String,
    pub table_size: String,
}

#[derive(Debug, Error)]
pub enum RepairError {
    #[error("an invalid index catalog query or repair statement failed")]
    Database(#[from] PostgresError),
    #[error("the migration coordinator lock for an index repair could not be acquired")]
    LockAcquire {
        #[source]
        source: PostgresError,
    },
    #[error("the migration coordinator lock for an index repair could not be released")]
    LockRelease {
        #[source]
        source: PostgresError,
    },
}

// Mirrors _invalid_litellm_indexes: no LiteLLM_ table index that is invalid and not owned by a constraint
pub async fn invalid_litellm_indexes<C>(
    client: &C,
    schema: &str,
) -> Result<Vec<InvalidIndex>, RepairError>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(INVALID_INDEXES_SQL, &[&schema, &LITELLM_TABLE_PATTERN])
        .await?;
    let mut indexes = Vec::with_capacity(rows.len());
    for row in &rows {
        indexes.push(InvalidIndex {
            schema: row.try_get(0)?,
            name: row.try_get(1)?,
            table_size: row.try_get(2)?,
        });
    }
    Ok(indexes)
}

// Mirrors _index_repair: an interrupted concurrent build leaves a *_ccnew/*_ccold index to drop; anything else is rebuilt
#[must_use]
pub fn index_repair(index: &InvalidIndex) -> (String, &'static str) {
    let target = format!(
        "\"{}\".\"{}\"",
        index.schema.replace('"', "\"\""),
        index.name.replace('"', "\"\"")
    );
    let transient =
        Regex::new(TRANSIENT_INDEX_SUFFIX).is_ok_and(|pattern| pattern.is_match(&index.name));
    if transient {
        return (
            format!("DROP INDEX CONCURRENTLY IF EXISTS {target}"),
            "Dropped leftover",
        );
    }
    (format!("REINDEX INDEX CONCURRENTLY {target}"), "Rebuilt")
}

async fn repair_index(client: &PostgresClient, index: &InvalidIndex) {
    let (statement, action) = index_repair(index);
    let qualified = format!("{}.{}", index.schema, index.name);
    if let Err(error) = client.batch_execute(&statement).await {
        tracing::warn!(
            error = %error,
            index = %qualified,
            statement = %statement,
            "could not repair an invalid index; it will be retried on the next database setup run, and the index owner can run the statement by hand"
        );
        return;
    }
    tracing::info!(action, index = %qualified, "repaired an invalid index");
}

// Mirrors _repair_under_migration_lock: re-check under the migration coordinator lock so a concurrent job is never fought
pub async fn repair_under_migration_lock(
    client: &PostgresClient,
    schema: &str,
    index: &InvalidIndex,
) -> Result<bool, RepairError> {
    let outcome = held_migration_lock(client, |acquired| async move {
        if !acquired {
            tracing::info!(
                "another process is building indexes under the migration lock; leaving the invalid index check to the next database setup run"
            );
            return Ok(false);
        }
        let still_invalid = invalid_litellm_indexes(client, schema).await?;
        let present = still_invalid
            .iter()
            .any(|found| found.schema == index.schema && found.name == index.name);
        if present {
            repair_index(client, index).await;
        }
        Ok(true)
    })
    .await;
    match outcome {
        Ok(repaired) => Ok(repaired),
        Err(HeldLockError::Body(error)) => Err(error),
        Err(HeldLockError::Acquire { source }) => Err(RepairError::LockAcquire { source }),
        Err(HeldLockError::Unlock { source }) => Err(RepairError::LockRelease { source }),
    }
}

// Mirrors repair_invalid_indexes: True when no invalid index remains, False when the repair is skipped or fails
pub async fn repair_invalid_indexes(check: &MigrationDriftCheck, lock_timeout: &str) -> bool {
    let Some(database_url) = &check.database_url else {
        return false;
    };
    let Ok(configured) = prisma_schema_param(database_url.expose_secret()) else {
        tracing::warn!(
            "the database URL could not be parsed for the invalid index repair; skipping it until the next database setup run"
        );
        return false;
    };
    let schema = configured.unwrap_or_else(|| DEFAULT_SCHEMA.to_owned());
    let repair_url = check.direct_url.as_ref().unwrap_or(database_url);
    let Ok(url) = strip_prisma_query_params(repair_url.expose_secret()) else {
        tracing::warn!(
            "the database URL could not be normalized for the invalid index repair; skipping it until the next database setup run"
        );
        return false;
    };
    let Ok(client) = connect(&SecretString::from(url)).await else {
        tracing::warn!(
            "the database connection for the invalid index repair could not be opened; retrying on the next database setup run"
        );
        return false;
    };
    match repair_over(&client, &schema, lock_timeout).await {
        Ok(repaired) => repaired,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "could not check for invalid indexes; retrying on the next database setup run"
            );
            false
        }
    }
}

async fn repair_over(
    client: &PostgresClient,
    schema: &str,
    lock_timeout: &str,
) -> Result<bool, RepairError> {
    client.batch_execute("SET statement_timeout = 0").await?;
    client
        .query_one(
            "SELECT set_config('lock_timeout', $1, false)",
            &[&lock_timeout],
        )
        .await?;
    let found = invalid_litellm_indexes(client, schema).await?;
    if found.is_empty() {
        return Ok(true);
    }
    tracing::warn!(
        count = found.len(),
        indexes = %describe(&found),
        "found invalid indexes left by an interrupted CREATE INDEX CONCURRENTLY; rebuilding"
    );
    let acquired: bool = client
        .query_one("SELECT pg_try_advisory_lock($1)", &[&INDEX_REPAIR_LOCK_KEY])
        .await?
        .get(0);
    if !acquired {
        tracing::info!("another replica is already rebuilding the invalid indexes; skipping");
        return Ok(false);
    }
    for index in &found {
        if !repair_under_migration_lock(client, schema, index).await? {
            return Ok(false);
        }
    }
    let remaining = invalid_litellm_indexes(client, schema).await?;
    Ok(remaining.is_empty())
}

fn describe(indexes: &[InvalidIndex]) -> String {
    indexes
        .iter()
        .map(|index| format!("{} (table size {})", index.name, index.table_size))
        .collect::<Vec<_>>()
        .join(", ")
}
