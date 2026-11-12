use std::future::Future;
use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::{Config, NoTls};
use uuid::Uuid;

use crate::migration_deploy::catalog::Migration;
use crate::migration_recovery::steps::{PgSteps, PgStepsInputs};
use crate::migration_recovery::tests::support::FakeJitter;

const TEST_TIMEOUT: Duration = Duration::from_secs(240);
pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct TestDatabase {
    pub pool: Pool,
    pub url: SecretString,
    pub schema: String,
}

impl TestDatabase {
    pub(super) fn steps<'input>(
        &self,
        catalog: &'input [Migration],
        jitter: &'input FakeJitter,
    ) -> PgSteps<'input> {
        PgSteps::new(PgStepsInputs {
            schema: self.schema.clone(),
            catalog,
            prisma_url: self.prisma_url(),
            deploy_url: self.url.clone(),
            lock_url: self.url.clone(),
            database_url: self.url.clone(),
            direct_url: Some(self.url.clone()),
            schema_prisma: &[],
            current_build: None,
            command_timeout: COMMAND_TIMEOUT,
            lock_timeout: LOCK_TIMEOUT,
            jitter,
            next_id,
        })
        .expect("the test database URL builds a deploy pool")
    }

    fn prisma_url(&self) -> SecretString {
        SecretString::from(format!(
            "{}&schema={}",
            self.url.expose_secret(),
            self.schema
        ))
    }
}

pub(super) fn next_id() -> Uuid {
    Uuid::new_v4()
}

pub(super) fn migration(name: &str, sql: &str) -> Migration {
    Migration {
        name: name.to_owned(),
        checksum: hex::encode(Sha256::digest(sql.as_bytes())),
        sql: sql.as_bytes().to_vec(),
    }
}

pub(super) async fn with_schema<F, Fut>(test: F)
where
    F: FnOnce(TestDatabase) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let fixture = PostgresFixture::provision_schema(test_url().as_ref())
        .resolve()
        .await
        .expect("the Postgres fixture is available");
    let pool = pool_for(&fixture);
    let schema: String = pool
        .get()
        .await
        .expect("a fixture connection")
        .query_one("SELECT current_schema()", &[])
        .await
        .expect("current_schema")
        .get(0);
    let database = TestDatabase {
        pool: pool.clone(),
        url: SecretString::from(fixture.options.url.expose_secret().to_owned()),
        schema,
    };
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        timeout(TEST_TIMEOUT, test(database))
            .await
            .expect("the test deadline");
    });
    let outcome = tasks.join_next().await.expect("the test task");
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
    outcome.expect("the test task joins");
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["migration-recovery-tests"])
        .url
        .map(SecretString::from)
}

fn pool_for(fixture: &PostgresFixture) -> Pool {
    let config: Config = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .expect("the fixture pool builds")
}

pub(super) fn ledger_table(schema: &str) -> String {
    format!("\"{}\".\"_prisma_migrations\"", schema.replace('"', "\"\""))
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

pub(super) async fn insert_row(database: &TestDatabase, row: &LedgerRow<'_>) {
    let statement = format!(
        "INSERT INTO {} (id, checksum, migration_name, logs, started_at, finished_at, \
         rolled_back_at, applied_steps_count) VALUES ($1, $2, $3, $4, \
         $5::text::timestamptz, CASE WHEN $6 THEN current_timestamp END, \
         CASE WHEN $7 THEN current_timestamp END, $8)",
        ledger_table(&database.schema)
    );
    database
        .pool
        .get()
        .await
        .expect("a fixture connection")
        .execute(
            &statement,
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
        .expect("the ledger row inserts");
}

pub(super) async fn insert_history(
    database: &TestDatabase,
    migration: &Migration,
    id: &str,
    started_at: &str,
    finished: bool,
) {
    insert_row(
        database,
        &LedgerRow {
            id,
            name: &migration.name,
            checksum: &migration.checksum,
            steps: 1,
            logs: if finished {
                None
            } else {
                Some("ERROR: deadlock detected")
            },
            finished,
            rolled_back: false,
            started_at,
        },
    )
    .await;
}

fn create_table_statement(schema: &str) -> String {
    format!(
        "CREATE TABLE {} (id varchar(36) PRIMARY KEY NOT NULL, checksum varchar(64) NOT NULL, \
         finished_at timestamptz, migration_name varchar(255) NOT NULL, logs text, \
         rolled_back_at timestamptz, started_at timestamptz NOT NULL DEFAULT now(), \
         applied_steps_count integer NOT NULL DEFAULT 0)",
        ledger_table(schema)
    )
}

pub(super) async fn create_ledger(database: &TestDatabase) {
    execute(database, &create_table_statement(&database.schema)).await;
}

pub(super) struct StoredRow {
    pub checksum: String,
    pub logs: Option<String>,
    pub steps: i32,
    pub started_at: String,
    pub finished: bool,
    pub rolled_back: bool,
}

pub(super) async fn stored_row(database: &TestDatabase, name: &str) -> StoredRow {
    let table = ledger_table(&database.schema);
    let statement = format!(
        "SELECT checksum, logs, applied_steps_count, started_at::text, \
         finished_at IS NOT NULL, rolled_back_at IS NOT NULL FROM {table} \
         WHERE migration_name = $1"
    );
    let row = database
        .pool
        .get()
        .await
        .expect("a fixture connection")
        .query_one(&statement, &[&name])
        .await
        .expect("the ledger row reads");
    StoredRow {
        checksum: row.get(0),
        logs: row.get(1),
        steps: row.get(2),
        started_at: row.get(3),
        finished: row.get(4),
        rolled_back: row.get(5),
    }
}

pub(super) async fn relation_exists(database: &TestDatabase, name: &str) -> bool {
    let lookup = database
        .pool
        .get()
        .await
        .expect("a fixture connection")
        .query_opt(
            "SELECT 1 FROM pg_class WHERE oid = to_regclass($1)",
            &[&name],
        )
        .await
        .expect("to_regclass reads");
    lookup.is_some()
}

pub(super) async fn execute(database: &TestDatabase, statement: &str) {
    database
        .pool
        .get()
        .await
        .expect("a fixture connection")
        .batch_execute(statement)
        .await
        .expect("the statement runs");
}

pub(super) async fn raised_database_error(
    database: &TestDatabase,
    message: &str,
) -> tokio_postgres::Error {
    database
        .pool
        .get()
        .await
        .expect("a fixture connection")
        .batch_execute(&format!("DO $$ BEGIN RAISE EXCEPTION '{message}'; END $$;"))
        .await
        .expect_err("the fixture raises the scripted error")
}
