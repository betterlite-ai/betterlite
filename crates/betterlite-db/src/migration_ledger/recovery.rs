use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio_postgres::Error as PostgresError;

use super::lock::{JitterSource, LockError, MigrationCoordinator};
use super::progress::{MigrationProgress, migration_records};
use crate::migration_deploy::catalog::Migration;

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("database operation failed")]
    Database(#[from] PostgresError),
    #[error("the migration lock could not be acquired")]
    Lock(#[from] LockError),
    #[error("Could not complete the confirmed migration history row; retry startup.")]
    CompletionMismatch,
}

// Mirrors recover_completed_migration: finish a proven successful row without erasing its durable evidence
pub async fn recover_completed_migration(
    coordinator: &MigrationCoordinator,
    schema: &str,
    migration: &Migration,
    jitter: &dyn JitterSource,
) -> Result<bool, RecoveryError> {
    coordinator.acquire_prisma_lock(jitter).await?;
    let records = migration_records(coordinator, schema, &migration.name).await?;
    let unfinished: Vec<&MigrationProgress> =
        records.iter().filter(|record| !record.finished).collect();
    if unfinished.is_empty() {
        let checksum = hex::encode(Sha256::digest(&migration.sql));
        return Ok(records.iter().any(|record| record.checksum == checksum));
    }
    let [progress] = unfinished.as_slice() else {
        return Ok(false);
    };
    if !progress.confirms_completion(&migration.sql) {
        return Ok(false);
    }
    let table = format!("\"{}\".\"_prisma_migrations\"", schema.replace('"', "\"\""));
    let statement = format!(
        "UPDATE {table} SET finished_at = current_timestamp \
         WHERE id = $1 AND checksum = $2 AND applied_steps_count = 1 \
         AND finished_at IS NULL AND rolled_back_at IS NULL AND coalesce(logs, '') = $3"
    );
    let affected = coordinator
        .client
        .execute(
            &statement,
            &[&progress.id, &progress.checksum, &progress.logs],
        )
        .await?;
    if affected != 1 {
        return Err(RecoveryError::CompletionMismatch);
    }
    tracing::info!(
        migration = %migration.name,
        "completed migration using its successful SQL step and matching checksum"
    );
    Ok(true)
}
