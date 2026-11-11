use std::str::{Utf8Error, from_utf8};

use regex::Regex;
use thiserror::Error;
use tokio_postgres::Error as PostgresError;

use super::lock::{JitterSource, LockError, MigrationCoordinator};
use super::progress::{MigrationProgress, migration_records};
use crate::migration_deploy::catalog::Migration;

const BLOCK_COMMENTS: &str = r"(?s)/\*.*?\*/";
const LINE_COMMENTS: &str = r"--[^\n]*";
const NO_OP_STATEMENT: &str = r"(?i)^\s*SELECT\s+1\s*$";

#[derive(Debug, Error)]
pub enum InertError {
    #[error("migration {name} is not valid UTF-8")]
    InvalidSql {
        name: String,
        #[source]
        source: Utf8Error,
    },
    #[error("database operation failed")]
    Database(#[from] PostgresError),
    #[error("the migration lock could not be acquired")]
    Lock(#[from] LockError),
    #[error(
        "Could not roll back the failed inert migration history row; rerun the database setup."
    )]
    RollbackMismatch,
}

// Mirrors is_inert_migration: comments and SELECT 1 change nothing, so replaying the file can neither repeat nor skip a change
#[must_use]
pub fn is_inert_migration(script: &str) -> bool {
    let (Ok(block), Ok(line), Ok(no_op)) = (
        Regex::new(BLOCK_COMMENTS),
        Regex::new(LINE_COMMENTS),
        Regex::new(NO_OP_STATEMENT),
    ) else {
        return false;
    };
    let without_blocks = block.replace_all(script, "");
    let stripped = line.replace_all(&without_blocks, "");
    stripped.split(';').all(|part| {
        let bare = part.trim();
        bare.is_empty() || no_op.is_match(bare)
    })
}

// Mirrors roll_back_failed_inert_migration: mark this build's inert file rolled back so the next deploy pass applies it
pub async fn rollback_inert_migration(
    coordinator: &MigrationCoordinator,
    schema: &str,
    migration: &Migration,
    jitter: &dyn JitterSource,
) -> Result<bool, InertError> {
    let script = from_utf8(&migration.sql).map_err(|source| InertError::InvalidSql {
        name: migration.name.clone(),
        source,
    })?;
    if !is_inert_migration(script) {
        return Ok(false);
    }
    coordinator.acquire_prisma_lock(jitter).await?;
    let records = migration_records(coordinator, schema, &migration.name).await?;
    let unfinished: Vec<&MigrationProgress> =
        records.iter().filter(|record| !record.finished).collect();
    let [progress] = unfinished.as_slice() else {
        return Ok(false);
    };
    let table = format!("\"{}\".\"_prisma_migrations\"", schema.replace('"', "\"\""));
    let statement = format!(
        "UPDATE {table} SET rolled_back_at = current_timestamp \
         WHERE id = $1 AND finished_at IS NULL AND rolled_back_at IS NULL"
    );
    let affected = coordinator
        .client
        .execute(&statement, &[&progress.id])
        .await?;
    if affected != 1 {
        return Err(InertError::RollbackMismatch);
    }
    tracing::info!(
        migration = %migration.name,
        "rolled back the failed history row of an inert migration"
    );
    Ok(true)
}
