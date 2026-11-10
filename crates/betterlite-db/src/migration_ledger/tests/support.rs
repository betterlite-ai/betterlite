use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::time::{Duration, Instant};

use deadpool_postgres::{Client, ClientWrapper, Pool};
use rstest::rstest;
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use tracing::subscriber::set_default;
use uuid::Uuid;

use super::baseline::{BaselineError, BaselineInputs, baseline_current_schema};
use super::environment::{migration_environment, prisma_schema_param};
use super::fixtures::{
    BodyError, DrawingJitter, LEDGER_DDL, LedgerRow, PUBLIC_SCHEMA, TestDatabase,
    acquired_coordinator, apply_compiled_history, apply_pinned_history, capture_logs, database,
    database_pair, exists, insert_row, ledger_rows, migration, pinned_manifest, read_contract,
    runtime_catalog, schema_prisma_bytes,
};
use super::inert::{InertError, is_inert_migration, rollback_inert_migration};
use super::ledger_reads::{failed_migration_logs, failed_migration_recovered};
use super::lock::{
    HeldLockError, LockError, MIGRATION_LOCK_KEY, MigrationCoordinator, PRISMA_LOCK_KEY,
    held_migration_lock,
};
use super::progress::MigrationProgress;
use super::recovery::{RecoveryError, recover_completed_migration};
use super::timeouts::{
    DEFAULT_COMMAND_TIMEOUT, DEFAULT_DEPLOY_TIMEOUT, DEFAULT_LOCK_TIMEOUT,
    PRISMA_COMMAND_TIMEOUT_ENV_VAR, prisma_migrate_deploy_timeout, timeout_from_env,
};
use crate::migration_deploy::catalog::Migration;
use crate::schema_contract::SchemaContract;

async fn connection(pool: &Pool) -> ClientWrapper {
    Client::take(pool.get().await.unwrap())
}

async fn create_ledger(pool: &Pool) {
    let client = pool.get().await.unwrap();
    client.batch_execute(LEDGER_DDL).await.unwrap();
}

fn pinned_catalog_map() -> BTreeMap<String, String> {
    pinned_manifest().into_iter().collect()
}

const FIXED_STARTED_AT: &str = "2025-01-01 00:00:00+00";
const LATER_STARTED_AT: &str = "2025-01-02 00:00:00+00";

fn progress(steps: i32, logs: &str, checksum: &str) -> MigrationProgress {
    MigrationProgress {
        id: "ledger-id".to_owned(),
        checksum: checksum.to_owned(),
        applied_steps_count: steps,
        logs: logs.to_owned(),
        finished: false,
    }
}

fn checksum_of(sql: &str) -> String {
    hex::encode(Sha256::digest(sql.as_bytes()))
}

fn baseline_input<'input>(
    catalog: &'input [Migration],
    schema_prisma: &'input [u8],
    current_build: Option<&'input SchemaContract>,
) -> BaselineInputs<'input> {
    BaselineInputs {
        schema: PUBLIC_SCHEMA,
        catalog,
        schema_prisma,
        current_build,
        command_timeout: Duration::from_secs(60),
        next_id: Uuid::new_v4,
    }
}

fn configured_url(database: &TestDatabase) -> &SecretString {
    &database.url
}

async fn insert_failed(pool: &Pool, id: &str, migration: &Migration, started_at: &str) {
    insert_row(
        pool,
        &LedgerRow {
            id,
            name: &migration.name,
            checksum: &migration.checksum,
            steps: 1,
            logs: Some("boom"),
            finished: false,
            rolled_back: false,
            started_at,
        },
    )
    .await;
}

async fn insert_finished(pool: &Pool, id: &str, migration: &Migration, started_at: &str) {
    insert_row(
        pool,
        &LedgerRow {
            id,
            name: &migration.name,
            checksum: &migration.checksum,
            steps: 1,
            logs: Some(""),
            finished: true,
            rolled_back: false,
            started_at,
        },
    )
    .await;
}

async fn insert_confirming(pool: &Pool, id: &str, migration: &Migration, started_at: &str) {
    insert_row(
        pool,
        &LedgerRow {
            id,
            name: &migration.name,
            checksum: &migration.checksum,
            steps: 1,
            logs: Some(""),
            finished: false,
            rolled_back: false,
            started_at,
        },
    )
    .await;
}

async fn insert_read(pool: &Pool, id: &str, name: &str, logs: Option<&str>, finished: bool) {
    insert_row(
        pool,
        &LedgerRow {
            id,
            name,
            checksum: &checksum_of(name),
            steps: 1,
            logs,
            finished,
            rolled_back: false,
            started_at: FIXED_STARTED_AT,
        },
    )
    .await;
}

async fn insert_reading_rows(pool: &Pool) {
    insert_read(pool, "row-null", "20250107000000_null", None, false).await;
    insert_read(pool, "row-blank", "20250108000000_blank", Some(""), false).await;
    insert_read(
        pool,
        "row-failed",
        "20250109000000_failed",
        Some("boom"),
        false,
    )
    .await;
}

async fn insert_read_later(pool: &Pool, id: &str, name: &str, logs: Option<&str>) {
    insert_row(
        pool,
        &LedgerRow {
            id,
            name,
            checksum: &checksum_of(name),
            steps: 1,
            logs,
            finished: true,
            rolled_back: false,
            started_at: LATER_STARTED_AT,
        },
    )
    .await;
}

async fn xact_probe(client: &ClientWrapper, key: i64) -> bool {
    client
        .query_one("SELECT pg_try_advisory_xact_lock($1) AS acquired", &[&key])
        .await
        .unwrap()
        .get(0)
}

async fn session_probe(client: &ClientWrapper, key: i64) -> bool {
    client
        .query_one("SELECT pg_try_advisory_lock($1) AS acquired", &[&key])
        .await
        .unwrap()
        .get(0)
}

async fn session_unlock(client: &ClientWrapper, key: i64) {
    client
        .execute("SELECT pg_advisory_unlock($1)", &[&key])
        .await
        .unwrap();
}
