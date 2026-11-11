#![cfg(test)]

use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::path::Path;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio::fs;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::{Client as PostgresClient, Config, NoTls};
use tracing::dispatcher::Dispatch;
use tracing_subscriber::fmt::MakeWriter;

use crate::migration_drift::MigrationDriftCheck;
use crate::schema_agent_identity::MIGRATION_CATALOG as AGENT_IDENTITY;
use crate::schema_baseline::MIGRATION_CATALOG as BASELINE;
use crate::schema_budget_windows::MIGRATION_CATALOG as BUDGET_WINDOWS;
use crate::schema_contract::SchemaContract;
use crate::schema_daily_usage::MIGRATION_CATALOG as DAILY_USAGE;
use crate::schema_identity_history::MIGRATION_CATALOG as IDENTITY_HISTORY;
use crate::schema_managed_resources::MIGRATION_CATALOG as MANAGED_RESOURCES;
use crate::schema_policy_resources::MIGRATION_CATALOG as POLICY_RESOURCES;
use crate::schema_routing_state::MIGRATION_CATALOG as ROUTING_STATE;

pub(super) const PUBLIC_SCHEMA: &str = "public";
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(60);
const TEST_TIMEOUT: Duration = Duration::from_secs(240);

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct TestDatabase {
    pub(super) pool: Pool,
    pub(super) schema: String,
    pub(super) url: SecretString,
    pub(super) direct_url: SecretString,
}

impl TestDatabase {
    pub(super) fn check(&self) -> MigrationDriftCheck {
        MigrationDriftCheck {
            database_url: Some(self.url.clone()),
            direct_url: Some(self.direct_url.clone()),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(super) fn joined(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
    }
}

impl Write for CapturedLogs {
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

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture() -> (CapturedLogs, Dispatch) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    (logs, Dispatch::new(subscriber))
}

// A unique schema per test: the fixture creates it and drops it with everything the test added
pub(super) async fn with_schema<F, Fut>(test: F)
where
    F: FnOnce(TestDatabase) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (fixture, pool, database) = schema_fixture().await;
    run(test(database), vec![pool], vec![fixture]).await;
}

// The pinned baseline SQL guards foreign-key names database-wide, so each independent history needs its own database
pub(super) async fn with_databases<F, Fut>(count: usize, test: F)
where
    F: FnOnce(Vec<TestDatabase>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut fixtures = Vec::with_capacity(count);
    let mut pools = Vec::with_capacity(count);
    let mut databases = Vec::with_capacity(count);
    for _ in 0..count {
        let fixture = PostgresFixture::provision_with_name_generator(
            test_url().as_ref(),
            &FixtureSeed::default(),
            &mut RandomDatabaseName,
        )
        .resolve()
        .await
        .unwrap();
        let pool = pool_for(&fixture);
        let url = SecretString::from(fixture.options.url.expose_secret().to_owned());
        databases.push(TestDatabase {
            pool: pool.clone(),
            url: url.clone(),
            direct_url: url,
            schema: PUBLIC_SCHEMA.to_owned(),
        });
        pools.push(pool);
        fixtures.push(fixture);
    }
    run(test(databases), pools, fixtures).await;
}

async fn schema_fixture() -> (PostgresFixture, Pool, TestDatabase) {
    let fixture = PostgresFixture::provision_schema(test_url().as_ref())
        .resolve()
        .await
        .unwrap();
    let pool = pool_for(&fixture);
    let schema: String = pool
        .get()
        .await
        .unwrap()
        .query_one("SELECT current_schema()", &[])
        .await
        .unwrap()
        .get(0);
    let direct_url = fixture.options.url.expose_secret().to_owned();
    let database = TestDatabase {
        pool: pool.clone(),
        url: SecretString::from(format!("{direct_url}&schema={schema}")),
        direct_url: SecretString::from(direct_url),
        schema,
    };
    (fixture, pool, database)
}

async fn run<Fut>(test: Fut, pools: Vec<Pool>, fixtures: Vec<PostgresFixture>)
where
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { timeout(TEST_TIMEOUT, test).await.unwrap() });
    let outcome = tasks.join_next().await.unwrap();
    for pool in pools {
        pool.close();
    }
    for fixture in fixtures {
        fixture.close_async().await.unwrap();
    }
    outcome.unwrap();
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["migration-drift-tests"])
        .url
        .map(SecretString::from)
}

fn pool_for(fixture: &PostgresFixture) -> Pool {
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .unwrap()
}

const SQL_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/schema_contract/fixtures/sql"
);

// The fixture seeds a partial baseline for its row seeds; a full history starts from an empty public schema
pub(super) async fn reset_public(pool: &Pool) {
    let reset = "DROP SCHEMA public CASCADE; CREATE SCHEMA public";
    let client = pool.get().await.unwrap();
    client.batch_execute(reset).await.unwrap();
}

pub(super) async fn apply_pinned(pool: &Pool) {
    let manifest = include_str!("../schema_contract/fixtures/sql.sha256");
    let client = pool.get().await.unwrap();
    for line in manifest.lines() {
        let (digest, file) = line.split_once("  ").unwrap();
        let bytes = fs::read(Path::new(SQL_DIR).join(file)).await.unwrap();
        assert_eq!(hex::encode(Sha256::digest(&bytes)), digest);
        apply_script(&client, &bytes).await;
    }
}

pub(super) async fn apply_compiled(pool: &Pool) {
    let client = pool.get().await.unwrap();
    for catalog in [
        BASELINE,
        DAILY_USAGE,
        MANAGED_RESOURCES,
        IDENTITY_HISTORY,
        POLICY_RESOURCES,
        ROUTING_STATE,
        BUDGET_WINDOWS,
        AGENT_IDENTITY,
    ] {
        for migration in catalog.files {
            apply_script(&client, migration.bytes).await;
        }
    }
}

async fn apply_script(client: &PostgresClient, bytes: &[u8]) {
    client
        .batch_execute(str::from_utf8(bytes).unwrap())
        .await
        .unwrap();
}

pub(super) async fn read_contract(pool: &Pool, schema: &str) -> SchemaContract {
    let client = pool.get().await.unwrap();
    SchemaContract::read(&**client, schema, READ_TIMEOUT)
        .await
        .unwrap()
}
