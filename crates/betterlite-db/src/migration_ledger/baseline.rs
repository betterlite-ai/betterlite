use std::time::Duration;

use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio_postgres::Error as PostgresError;
use uuid::Uuid;

use super::lock::{JitterSource, LockError, MigrationCoordinator};
use crate::migration_deploy::catalog::Migration;
use crate::schema_agent_identity::MIGRATION_CATALOG as AGENT_IDENTITY;
use crate::schema_baseline::MIGRATION_CATALOG as BASELINE;
use crate::schema_baseline::MigrationCatalog;
use crate::schema_budget_windows::MIGRATION_CATALOG as BUDGET_WINDOWS;
use crate::schema_contract::comparison::Comparison;
use crate::schema_contract::{ContractError, SchemaContract};
use crate::schema_daily_usage::MIGRATION_CATALOG as DAILY_USAGE;
use crate::schema_identity_history::MIGRATION_CATALOG as IDENTITY_HISTORY;
use crate::schema_managed_resources::MIGRATION_CATALOG as MANAGED_RESOURCES;
use crate::schema_policy_resources::MIGRATION_CATALOG as POLICY_RESOURCES;
use crate::schema_routing_state::MIGRATION_CATALOG as ROUTING_STATE;

// The three package copies of schema.prisma are byte-identical, so one sum identifies the build's datamodel
const PACKAGED_SCHEMA_SHA256: &str =
    "a17e69687d14d78dc7cee49770f1c1e8b2726cd8823d37c6dff48ae151533342";

const PACKAGED_CATALOGS: [MigrationCatalog; 8] = [
    BASELINE,
    DAILY_USAGE,
    MANAGED_RESOURCES,
    IDENTITY_HISTORY,
    POLICY_RESOURCES,
    ROUTING_STATE,
    BUDGET_WINDOWS,
    AGENT_IDENTITY,
];

#[derive(Debug, Error)]
pub enum BaselineError {
    #[error("Cannot automatically baseline an existing database with custom migration history.")]
    CustomHistory,
    #[error("database operation failed")]
    Database(#[from] PostgresError),
    #[error("the migration lock could not be acquired")]
    Lock(#[from] LockError),
    #[error(
        "this build's schema is unavailable, so the existing schema was not verified and no reconciliation was performed"
    )]
    VerificationUnavailable,
    #[error("the existing schema could not be read for verification")]
    Verification(#[from] ContractError),
    #[error("the existing database schema does not match this build: {summary}")]
    SchemaMismatch {
        summary: String,
        comparison: Comparison,
    },
}

pub struct BaselineInputs<'input> {
    pub schema: &'input str,
    // the catalog offered for adoption, read with B873's migration_files from the runtime directory
    pub catalog: &'input [Migration],
    pub schema_prisma: &'input [u8],
    // B701's catalog of the independently initialized current build; absent when it cannot be verified
    pub current_build: Option<&'input SchemaContract>,
    pub command_timeout: Duration,
    pub next_id: fn() -> Uuid,
}

#[must_use]
fn packaged_catalog() -> Vec<(&'static str, &'static str)> {
    let mut packaged = Vec::new();
    for catalog in PACKAGED_CATALOGS {
        for file in catalog.files {
            packaged.push((file.directory, file.sha256));
        }
    }
    packaged.sort_unstable();
    packaged
}

// Mirrors baseline_current_schema: adopt an already-current schema only after proving it is this build's schema
pub async fn baseline_current_schema(
    coordinator: &MigrationCoordinator,
    inputs: &BaselineInputs<'_>,
    jitter: &dyn JitterSource,
) -> Result<(), BaselineError> {
    require_packaged_history(inputs)?;
    coordinator.acquire_prisma_lock(jitter).await?;
    let table = format!(
        "\"{}\".\"_prisma_migrations\"",
        inputs.schema.replace('"', "\"\"")
    );
    if ledger_exists(coordinator, &table).await? {
        return Ok(());
    }
    verify_against_current_build(coordinator, inputs).await?;
    coordinator.check_connection().await?;
    create_ledger(coordinator, &table, inputs).await?;
    tracing::warn!(
        migrated = inputs.catalog.len(),
        "Legacy migration history was missing. The existing Prisma schema matches this build; \
         adopted the packaged migrations as a baseline. No schema changes were applied, and \
         historical data backfills were not replayed or verified. Continuing startup; review any \
         feature-specific backfill requirements."
    );
    Ok(())
}

// Refuses an empty or custom migration history and a datamodel that is not the packaged build's
fn require_packaged_history(inputs: &BaselineInputs<'_>) -> Result<(), BaselineError> {
    let mut offered: Vec<(&str, &str)> = inputs
        .catalog
        .iter()
        .map(|migration| (migration.name.as_str(), migration.checksum.as_str()))
        .collect();
    offered.sort_unstable();
    let datamodel_matches =
        hex::encode(Sha256::digest(inputs.schema_prisma)) == PACKAGED_SCHEMA_SHA256;
    if offered.is_empty() || offered != packaged_catalog() || !datamodel_matches {
        return Err(BaselineError::CustomHistory);
    }
    Ok(())
}

async fn ledger_exists(
    coordinator: &MigrationCoordinator,
    table: &str,
) -> Result<bool, PostgresError> {
    let existing = coordinator
        .client
        .query_opt("SELECT to_regclass($1)::text", &[&table])
        .await?;
    let Some(row) = existing else {
        return Ok(false);
    };
    let found: Option<String> = row.try_get(0)?;
    Ok(found.is_some())
}

async fn verify_against_current_build(
    coordinator: &MigrationCoordinator,
    inputs: &BaselineInputs<'_>,
) -> Result<(), BaselineError> {
    let Some(current_build) = inputs.current_build else {
        return Err(BaselineError::VerificationUnavailable);
    };
    let live =
        SchemaContract::read(&*coordinator.client, inputs.schema, inputs.command_timeout).await?;
    let comparison = current_build.compare(&live);
    if comparison.differences.is_empty() {
        return Ok(());
    }
    Err(BaselineError::SchemaMismatch {
        summary: mismatch_summary(&comparison),
        comparison,
    })
}

async fn create_ledger(
    coordinator: &MigrationCoordinator,
    table: &str,
    inputs: &BaselineInputs<'_>,
) -> Result<(), PostgresError> {
    let create = format!(
        "CREATE TABLE {table} (id varchar(36) PRIMARY KEY NOT NULL, checksum varchar(64) NOT NULL, \
         finished_at timestamptz, migration_name varchar(255) NOT NULL, logs text, \
         rolled_back_at timestamptz, started_at timestamptz NOT NULL DEFAULT now(), \
         applied_steps_count integer NOT NULL DEFAULT 0)"
    );
    coordinator.client.execute(&create, &[]).await?;
    let insert = format!(
        "INSERT INTO {table} (id, checksum, migration_name, logs, started_at, finished_at) \
         VALUES ($1, $2, $3, '', current_timestamp, current_timestamp)"
    );
    for migration in inputs.catalog {
        let id = (inputs.next_id)().to_string();
        coordinator
            .client
            .execute(&insert, &[&id, &migration.checksum, &migration.name])
            .await?;
    }
    Ok(())
}

fn mismatch_summary(comparison: &Comparison) -> String {
    let first: Vec<String> = comparison
        .differences
        .iter()
        .take(3)
        .map(|(fact, change)| format!("{}.{} {change:?}", fact.relation, fact.member))
        .collect();
    first.join(", ")
}
