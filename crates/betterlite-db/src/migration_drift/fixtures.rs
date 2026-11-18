#![cfg(test)]

use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::{Config, NoTls};
use tracing::dispatcher::Dispatch;
use tracing_subscriber::fmt::MakeWriter;

use crate::migration_drift::MigrationDriftCheck;
use crate::schema_contract::SchemaContract;

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

// An empty database per test, dropped with everything the test added
pub(super) async fn with_schema<F, Fut>(test: F)
where
    F: FnOnce(TestDatabase) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (fixture, pool, database) = database(Vec::new()).await;
    run(test(database), vec![pool], vec![fixture]).await;
}

// One database per statement list: the pinned baseline SQL guards foreign-key names database-wide
pub(super) async fn with_databases<F, Fut>(templates: Vec<Vec<String>>, test: F)
where
    F: FnOnce(Vec<TestDatabase>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut fixtures = Vec::with_capacity(templates.len());
    let mut pools = Vec::with_capacity(templates.len());
    let mut databases = Vec::with_capacity(templates.len());
    for statements in templates {
        let (fixture, pool, test_database) = database(statements).await;
        databases.push(test_database);
        pools.push(pool);
        fixtures.push(fixture);
    }
    run(test(databases), pools, fixtures).await;
}

async fn database(statements: Vec<String>) -> (PostgresFixture, Pool, TestDatabase) {
    let fixture = PostgresFixture::provision_with_statements(test_url().as_ref(), statements)
        .resolve()
        .await
        .unwrap();
    let pool = pool_for(&fixture);
    let url = SecretString::from(fixture.options.url.expose_secret().to_owned());
    let database = TestDatabase {
        pool: pool.clone(),
        url: url.clone(),
        direct_url: url,
        schema: PUBLIC_SCHEMA.to_owned(),
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

pub(super) async fn read_contract(pool: &Pool, schema: &str) -> SchemaContract {
    let client = pool.get().await.unwrap();
    SchemaContract::read(&**client, schema, READ_TIMEOUT)
        .await
        .unwrap()
}
