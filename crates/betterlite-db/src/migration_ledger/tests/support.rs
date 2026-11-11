use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::iter::repeat_with;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use deadpool_postgres::{Client, ClientWrapper, Pool};
use rstest::rstest;
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use tokio::time::Instant as TokioInstant;
use tracing::Subscriber;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use super::baseline::{BaselineError, BaselineInputs, baseline_current_schema};
use super::budget::{MAX_MIGRATE_DEPLOY_ATTEMPTS, MigrateAttemptBudget};
use super::catalog::{get_migration_names, max_migration_timestamp, migration_timestamp};
use super::environment::{migration_environment, prisma_schema_param};
use super::fixtures::{
    DrawingJitter, HistorySource, LEDGER_DDL, LedgerRow, PUBLIC_SCHEMA, TestDatabase,
    acquired_coordinator, apply_history, exists, harness, insert_row, ledger_rows, migration,
    pinned_manifest, read_contract, runtime_catalog, schema_prisma_bytes,
};
use super::inert::{InertError, is_inert_migration, rollback_inert_migration};
use super::ledger_reads::{FailedLookup, FailedMigration, look_up_failed_migration};
use super::lock::{
    HeldLockError, JitterSource as _, LockError, MIGRATION_LOCK_KEY, MigrationCoordinator,
    PRISMA_LOCK_KEY, SystemJitter, held_migration_lock, pause,
};
use super::progress::MigrationProgress;
use super::recovery::{RecoveryError, recover_completed_migration};
use super::timeouts::{
    DEFAULT_COMMAND_TIMEOUT, DEFAULT_DEPLOY_TIMEOUT, DEFAULT_LOCK_TIMEOUT,
    PRISMA_COMMAND_TIMEOUT_ENV_VAR, prisma_migrate_deploy_timeout, timeout_from_env,
};
use crate::migration_deploy::catalog::Migration;
use crate::schema_contract::SchemaContract;

async fn database<F, Fut>(test: F)
where
    F: FnOnce(TestDatabase) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    harness(1, move |mut databases| test(databases.remove(0))).await;
}

// The live database and the independently initialized reference database a baseline verifies against
async fn database_pair<F, Fut>(test: F)
where
    F: FnOnce(TestDatabase, TestDatabase) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    harness(2, move |mut databases| {
        let reference = databases.remove(1);
        test(databases.remove(0), reference)
    })
    .await;
}

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

fn row<'row>(id: &'row str, name: &'row str, checksum: &'row str) -> LedgerRow<'row> {
    LedgerRow {
        id,
        name,
        checksum,
        steps: 1,
        logs: Some("boom"),
        finished: false,
        rolled_back: false,
        started_at: FIXED_STARTED_AT,
    }
}

async fn insert_failed(pool: &Pool, id: &str, migration: &Migration, started_at: &str) {
    let spec = row(id, &migration.name, &migration.checksum);
    insert_row(pool, &LedgerRow { started_at, ..spec }).await;
}

async fn insert_confirmed(pool: &Pool, id: &str, migration: &Migration, finished: bool) {
    let spec = row(id, &migration.name, &migration.checksum);
    insert_row(
        pool,
        &LedgerRow {
            logs: Some(""),
            finished,
            ..spec
        },
    )
    .await;
}

async fn insert_read(pool: &Pool, id: &str, name: &str, logs: Option<&str>, resolved: bool) {
    let checksum = checksum_of(name);
    let spec = row(id, name, &checksum);
    let ledger = LedgerRow {
        logs,
        finished: resolved,
        started_at: if resolved {
            LATER_STARTED_AT
        } else {
            FIXED_STARTED_AT
        },
        ..spec
    };
    insert_row(pool, &ledger).await;
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

#[derive(Clone, Default)]
pub(super) struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    pub(super) fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
    }
}

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogCapture {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogCapture, impl Subscriber + Send + Sync) {
    let logs = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}

#[derive(Debug, thiserror::Error)]
pub(super) enum BodyError {
    #[error("the migration lock body refused")]
    Refused,
}
