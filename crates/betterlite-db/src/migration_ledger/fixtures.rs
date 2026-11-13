#![cfg(test)]

use std::fs;
use std::future::Future;
use std::path::Path;
use std::str;
use std::time::Duration;

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::{NoTls, Row};

use crate::migration_deploy::catalog::{Migration, migration_files};
use crate::migration_ledger::lock::{JitterSource, MigrationCoordinator};
use crate::schema_agent_identity::MIGRATION_CATALOG as AGENT_IDENTITY;
use crate::schema_baseline::MIGRATION_CATALOG as BASELINE;
use crate::schema_baseline::MigrationCatalog;
use crate::schema_budget_windows::MIGRATION_CATALOG as BUDGET_WINDOWS;
use crate::schema_contract::SchemaContract;
use crate::schema_daily_usage::MIGRATION_CATALOG as DAILY_USAGE;
use crate::schema_identity_history::MIGRATION_CATALOG as IDENTITY_HISTORY;
use crate::schema_managed_resources::MIGRATION_CATALOG as MANAGED_RESOURCES;
use crate::schema_policy_resources::MIGRATION_CATALOG as POLICY_RESOURCES;
use crate::schema_routing_state::MIGRATION_CATALOG as ROUTING_STATE;

pub(super) const PUBLIC_SCHEMA: &str = "public";
const TEST_TIMEOUT: Duration = Duration::from_secs(240);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) const LEDGER_DDL: &str = "CREATE TABLE _prisma_migrations (id varchar(36) PRIMARY KEY \
    NOT NULL, checksum varchar(64) NOT NULL, finished_at timestamptz, migration_name varchar(255) \
    NOT NULL, logs text, rolled_back_at timestamptz, started_at timestamptz NOT NULL DEFAULT now(), \
    applied_steps_count integer NOT NULL DEFAULT 0)";

const PINNED_CATALOGS: [MigrationCatalog; 8] = [
    BASELINE,
    DAILY_USAGE,
    MANAGED_RESOURCES,
    IDENTITY_HISTORY,
    POLICY_RESOURCES,
    ROUTING_STATE,
    BUDGET_WINDOWS,
    AGENT_IDENTITY,
];

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct DrawingJitter(pub u64);

impl JitterSource for DrawingJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        self.0.min(upper_exclusive_nanos.saturating_sub(1))
    }
}

#[derive(Clone)]
pub(super) struct TestDatabase {
    pub pool: Pool,
    pub url: SecretString,
}

pub(super) async fn harness<F, Fut>(count: usize, test: F)
where
    F: FnOnce(Vec<TestDatabase>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut fixtures = Vec::new();
    let mut databases = Vec::new();
    for _ in 0..count {
        let (fixture, database) = provision().await;
        fixtures.push(fixture);
        databases.push(database);
    }
    let pools: Vec<Pool> = databases
        .iter()
        .map(|database| database.pool.clone())
        .collect();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { timeout(TEST_TIMEOUT, test(databases)).await.unwrap() });
    let result = tasks.join_next().await.unwrap();
    for pool in pools {
        pool.close();
    }
    for fixture in fixtures {
        fixture.close_async().await.unwrap();
    }
    result.unwrap();
}

async fn provision() -> (PostgresFixture, TestDatabase) {
    let url = Environment::parse_from(["migration-ledger-tests"])
        .url
        .map(SecretString::from);
    let mut task = PostgresFixture::provision_with_name_generator(
        url.as_ref(),
        &FixtureSeed::default(),
        &mut RandomDatabaseName,
    );
    let fixture = task.resolve().await.unwrap();
    let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
        .max_size(4)
        .build()
        .unwrap();
    let database_url = SecretString::from(fixture.options.url.expose_secret().to_owned());
    (
        fixture,
        TestDatabase {
            pool,
            url: database_url,
        },
    )
}

pub(super) async fn acquired_coordinator(
    pool: &Pool,
    lock_timeout: Duration,
) -> MigrationCoordinator {
    let client = Client::take(pool.get().await.unwrap());
    MigrationCoordinator::acquire(client, lock_timeout, &DrawingJitter(0))
        .await
        .unwrap()
}

pub(super) fn migration(name: &str, sql: &str) -> Migration {
    Migration {
        name: name.to_owned(),
        checksum: hex::encode(Sha256::digest(sql.as_bytes())),
        sql: sql.as_bytes().to_vec(),
    }
}

pub(super) struct LedgerRow<'row> {
    pub id: &'row str,
    pub name: &'row str,
    pub checksum: &'row str,
    pub steps: i32,
    pub logs: Option<&'row str>,
    pub finished: bool,
    pub rolled_back: bool,
    pub started_at: &'row str,
}

pub(super) async fn insert_row(pool: &Pool, row: &LedgerRow<'_>) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO _prisma_migrations (id, checksum, migration_name, logs, started_at, \
             finished_at, rolled_back_at, applied_steps_count) VALUES ($1, $2, $3, $4, \
             $5::text::timestamptz, CASE WHEN $6 THEN current_timestamp END, \
             CASE WHEN $7 THEN current_timestamp END, $8)",
            &[
                &row.id,
                &row.checksum,
                &row.name,
                &row.logs,
                &row.started_at,
                &row.finished,
                &row.rolled_back,
                &row.steps,
            ],
        )
        .await
        .unwrap();
}

pub(super) async fn ledger_rows(pool: &Pool) -> Vec<Row> {
    pool.get()
        .await
        .unwrap()
        .query(
            "SELECT id, checksum, migration_name, logs, applied_steps_count, \
             finished_at IS NOT NULL AS finished, rolled_back_at IS NOT NULL AS rolled_back, \
             started_at, finished_at = started_at AS equal_times \
             FROM _prisma_migrations ORDER BY migration_name",
            &[],
        )
        .await
        .unwrap()
}

pub(super) async fn exists(pool: &Pool, name: &str) -> bool {
    let found: Option<String> = pool
        .get()
        .await
        .unwrap()
        .query_one("SELECT to_regclass($1)::text", &[&name])
        .await
        .unwrap()
        .get(0);
    found.is_some()
}

pub(super) async fn read_contract(pool: &Pool) -> SchemaContract {
    let client = pool.get().await.unwrap();
    SchemaContract::read(&**client, PUBLIC_SCHEMA, READ_TIMEOUT)
        .await
        .unwrap()
}

pub(super) async fn runtime_catalog() -> Vec<Migration> {
    migration_files(Path::new(env!("CARGO_MANIFEST_DIR")))
        .await
        .unwrap()
}

pub(super) fn schema_prisma_bytes() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/schema_contract/fixtures/extras/schema.prisma"),
    )
    .unwrap()
}

pub(super) fn pinned_manifest() -> Vec<(String, String)> {
    include_str!("../schema_contract/fixtures/sql.sha256")
        .lines()
        .map(|line| {
            let (sum, file) = line.split_once("  ").unwrap();
            (file.trim_end_matches(".sql").to_owned(), sum.to_owned())
        })
        .collect()
}

#[derive(Clone, Copy)]
pub(super) enum HistorySource {
    // the pinned SQL files, a source independent of the compiled catalog
    Pinned,
    Compiled,
}

pub(super) async fn apply_history(pool: &Pool, source: HistorySource) {
    let statements = match source {
        HistorySource::Pinned => pinned_statements(),
        HistorySource::Compiled => compiled_statements(),
    };
    let client = pool.get().await.unwrap();
    for statement in statements {
        client
            .batch_execute(str::from_utf8(&statement).unwrap())
            .await
            .unwrap();
    }
}

fn pinned_statements() -> Vec<Vec<u8>> {
    pinned_manifest()
        .into_iter()
        .map(|(name, _sum)| {
            fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src/schema_contract/fixtures/sql")
                    .join(format!("{name}.sql")),
            )
            .unwrap()
        })
        .collect()
}

fn compiled_statements() -> Vec<Vec<u8>> {
    PINNED_CATALOGS
        .iter()
        .flat_map(|catalog| catalog.files.iter())
        .map(|file| file.bytes.to_vec())
        .collect()
}
