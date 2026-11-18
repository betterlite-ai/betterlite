use std::future::Future;
use std::time::Duration;

use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_db::request_log_index_catalog::definitions::REQUEST_LOG_INDEXES;
use betterlite_postgres_fixture::connection::PostgresFixture;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::{Config, NoTls};

use crate::loader::startup::process_environment;
use crate::sections::database_startup::startup::{StartupSettings, StartupSwitch};

const TEST_TIMEOUT: Duration = Duration::from_secs(240);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const LOCK_TIMEOUT: Duration = Duration::from_secs(120);
const DEPLOY_TIMEOUT: Duration = Duration::from_secs(120);

// The retry pauses are not under test here, and a fixed zero keeps the ledger ids and passes reproducible
pub(super) struct NoJitter;

impl JitterSource for NoJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

#[derive(Clone)]
pub(super) struct SchemaDatabase {
    pool: Pool,
    url: SecretString,
    pub(super) schema: String,
}

impl SchemaDatabase {
    pub(super) fn settings(&self) -> StartupSettings {
        StartupSettings {
            updates_enabled: StartupSwitch(true),
            database_url: Some(self.url.clone()),
            direct_url: None,
            use_v2_migration_resolver: StartupSwitch(true),
            use_prisma_db_push: StartupSwitch(false),
            set_replica_identity_full: StartupSwitch(false),
            command_timeout: COMMAND_TIMEOUT,
            lock_timeout: LOCK_TIMEOUT,
            deploy_timeout: DEPLOY_TIMEOUT,
            bootstrap_inputs: Vec::new(),
        }
    }

    // The relreplident letter of the spend-log table: d is the default identity, f is full.
    pub(super) async fn spend_log_replica_identity(&self) -> String {
        self.pool
            .get()
            .await
            .expect("a fixture connection")
            .query_one(
                "SELECT c.relreplident::text FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = 'LiteLLM_SpendLogs'",
                &[&self.schema],
            )
            .await
            .expect("the replica identity reads")
            .get(0)
    }

    // The request-log indexes this schema actually carries, sorted to compare against the catalog.
    pub(super) async fn present_request_log_indexes(&self) -> Vec<String> {
        let names: Vec<&str> = REQUEST_LOG_INDEXES.iter().map(|index| index.name).collect();
        let client = self.pool.get().await.expect("a fixture connection");
        let mut present: Vec<String> = client
            .query(
                "SELECT indexname::text FROM pg_indexes \
                 WHERE schemaname = $1 AND indexname::text = ANY($2)",
                &[&self.schema, &names],
            )
            .await
            .expect("the index catalog reads")
            .iter()
            .map(|row| row.get(0))
            .collect();
        present.sort();
        present
    }
}

// The request-log indexes the catalog promises, sorted so a missing one shows in the diff.
pub(super) fn catalog_request_log_indexes() -> Vec<String> {
    let mut names: Vec<String> = REQUEST_LOG_INDEXES
        .iter()
        .map(|index| index.name.to_owned())
        .collect();
    names.sort();
    names
}

pub(super) async fn with_schema<F, Fut>(test: F)
where
    F: FnOnce(SchemaDatabase) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (database, fixture) = provision().await;
    let pool = database.pool.clone();
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

async fn provision() -> (SchemaDatabase, PostgresFixture) {
    let configured = process_environment()
        .get("POSTGRES_TEST_URL")
        .cloned()
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(configured.as_ref(), Vec::new())
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
    let database = SchemaDatabase {
        pool,
        url: SecretString::from(fixture.options.url.expose_secret().to_owned()),
        schema: "public".to_owned(),
    };
    (database, fixture)
}
