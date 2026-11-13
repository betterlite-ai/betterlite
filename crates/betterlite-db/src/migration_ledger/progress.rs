use sha2::{Digest as _, Sha256};
use tokio_postgres::Error as PostgresError;

use super::lock::MigrationCoordinator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationProgress {
    pub id: String,
    pub checksum: String,
    pub applied_steps_count: i32,
    pub logs: String,
    pub finished: bool,
}

impl MigrationProgress {
    // Mirrors MigrationProgress.confirms_completion: one successful step, whitespace-only logs and the file's own sha256
    #[must_use]
    pub fn confirms_completion(&self, script: &[u8]) -> bool {
        self.applied_steps_count == 1
            && self.logs.trim().is_empty()
            && self.checksum == hex::encode(Sha256::digest(script))
    }
}

// Mirrors _migration_records: every non-rolled-back ledger row of this migration in the selected namespace
pub(super) async fn migration_records(
    coordinator: &MigrationCoordinator,
    schema: &str,
    migration_name: &str,
) -> Result<Vec<MigrationProgress>, PostgresError> {
    let table = format!("\"{}\".\"_prisma_migrations\"", schema.replace('"', "\"\""));
    let statement = format!(
        "SELECT id, checksum, applied_steps_count, coalesce(logs, '') AS logs, \
         finished_at IS NOT NULL AS finished FROM {table} \
         WHERE migration_name = $1 AND rolled_back_at IS NULL"
    );
    let rows = coordinator
        .client
        .query(&statement, &[&migration_name])
        .await?;
    rows.iter()
        .map(|row| {
            Ok(MigrationProgress {
                id: row.try_get(0)?,
                checksum: row.try_get(1)?,
                applied_steps_count: row.try_get(2)?,
                logs: row.try_get(3)?,
                finished: row.try_get(4)?,
            })
        })
        .collect()
}
