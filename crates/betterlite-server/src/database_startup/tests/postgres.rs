use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::database_startup::startup::StartupSettings;
use betterlite_db::migration_deploy::catalog::Migration;
use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::{Config, NoTls};

const TEST_TIMEOUT: Duration = Duration::from_secs(240);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const LOCK_TIMEOUT: Duration = Duration::from_secs(120);
const DEPLOY_TIMEOUT: Duration = Duration::from_secs(120);

// The retry pauses are not under test here, and a fixed zero keeps the ledger ids and passes reproducible
pub(crate) struct NoJitter;

impl JitterSource for NoJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

#[derive(Clone)]
pub(crate) struct SchemaDatabase {
    pub(crate) pool: Pool,
    pub(crate) url: SecretString,
    pub(crate) schema: String,
}

impl SchemaDatabase {
    pub(crate) fn settings(&self) -> StartupSettings {
        StartupSettings {
            updates_enabled: true,
            database_url: Some(SecretString::from(format!(
                "{}&schema={}",
                self.url.expose_secret(),
                self.schema
            ))),
            direct_url: None,
            use_v2_migration_resolver: true,
            use_prisma_db_push: false,
            set_replica_identity_full: false,
            command_timeout: COMMAND_TIMEOUT,
            lock_timeout: LOCK_TIMEOUT,
            deploy_timeout: DEPLOY_TIMEOUT,
            bootstrap_inputs: Vec::new(),
        }
    }
}

pub(crate) struct LedgerRow {
    pub(crate) name: String,
    pub(crate) checksum: String,
    pub(crate) finished: bool,
    pub(crate) rolled_back: bool,
    pub(crate) logs: Option<String>,
}

pub(crate) async fn with_schema<F, Fut>(test: F)
where
    F: FnOnce(SchemaDatabase) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let configured = Environment::parse_from(["database-startup-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(configured.as_ref())
        .resolve()
        .await
        .expect("the Postgres fixture is available");
    let config: Config = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .expect("the fixture pool builds");
    let schema: String = pool
        .get()
        .await
        .expect("a fixture connection")
        .query_one("SELECT current_schema()", &[])
        .await
        .expect("current_schema reads")
        .get(0);
    let database = SchemaDatabase {
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

pub(crate) fn migration(name: &str, sql: &str) -> Migration {
    Migration {
        name: name.to_owned(),
        checksum: hex::encode(Sha256::digest(sql.as_bytes())),
        sql: sql.as_bytes().to_vec(),
    }
}

pub(crate) async fn ledger_rows(database: &SchemaDatabase) -> Vec<LedgerRow> {
    let table = format!(
        "\"{}\".\"_prisma_migrations\"",
        database.schema.replace('"', "\"\"")
    );
    let client = database.pool.get().await.expect("a fixture connection");
    let rows = client
        .query(
            &format!(
                "SELECT migration_name, checksum, finished_at IS NOT NULL, \
                 rolled_back_at IS NOT NULL, logs FROM {table} ORDER BY migration_name"
            ),
            &[],
        )
        .await
        .expect("the migration ledger reads");
    rows.into_iter()
        .map(|row| LedgerRow {
            name: row.get(0),
            checksum: row.get(1),
            finished: row.get(2),
            rolled_back: row.get(3),
            logs: row.get(4),
        })
        .collect()
}

pub(crate) async fn table_exists(database: &SchemaDatabase, name: &str) -> bool {
    let table = format!("\"{}\".\"{name}\"", database.schema.replace('"', "\"\""));
    let client = database.pool.get().await.expect("a fixture connection");
    client
        .query_one("SELECT to_regclass($1) IS NOT NULL", &[&table])
        .await
        .expect("the table lookup runs")
        .get(0)
}

pub(crate) async fn row_count(database: &SchemaDatabase, table: &str) -> i64 {
    let qualified = format!("\"{}\".\"{table}\"", database.schema.replace('"', "\"\""));
    let client = database.pool.get().await.expect("a fixture connection");
    client
        .query_one(&format!("SELECT count(*) FROM {qualified}"), &[])
        .await
        .expect("the row count reads")
        .get(0)
}
