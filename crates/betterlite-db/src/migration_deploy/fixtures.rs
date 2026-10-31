#![cfg(test)]

use std::env::temp_dir;
use std::fs::remove_dir_all;
use std::future::{Future, poll_fn};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::id;
use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::fs;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep, timeout};
use tokio_postgres::{Client as PostgresClient, Config, Error as PostgresError, NoTls, Row};
use uuid::Uuid;

use super::catalog::Migration;
use super::{DeployError, MigrationDeploy};

static DATABASE_TESTS: Mutex<()> = Mutex::const_new(());

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct Database {
    pub pool: Pool,
    pub schema: String,
}

pub(super) async fn database<F, Fut>(test: F)
where
    F: FnOnce(Database) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let _guard = DATABASE_TESTS.lock().await;
    let test_file = acquire_test_file().await;
    let url = Environment::parse_from(["migration-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let lock_config = config.clone();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(5)
        .build()
        .unwrap();
    let (test_lock, mut connection_tasks) = lock_connection(lock_config).await;
    let context = context(pool.clone()).await;
    let custom = format!("{}\"';--", context.schema);
    let cleanup_sql = format!(
        "DROP SCHEMA IF EXISTS \"{}\" CASCADE",
        custom.replace('"', "\"\"")
    );
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        timeout(Duration::from_secs(20), test(context))
            .await
            .unwrap();
    });
    let result = tasks.join_next().await.unwrap();
    clear_deployment_lock(&pool).await;
    release_lock(test_lock, &mut connection_tasks).await;
    let cleanup = pool.get().await.unwrap().batch_execute(&cleanup_sql).await;
    pool.close();
    fixture.close_async().await.unwrap();
    cleanup.unwrap();
    result.unwrap();
    fs::remove_file(test_file).await.unwrap();
}

async fn acquire_test_file() -> PathBuf {
    let path = temp_dir().join("betterlite-migration-deploy.lock");
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => {
                drop(file);
                fs::write(&path, id().to_string()).await.unwrap();
                return path;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let owner = fs::read_to_string(&path).await.unwrap_or_default();
                let process_path = PathBuf::from(format!("/proc/{}", owner.trim()));
                let process_exists = fs::try_exists(process_path).await.unwrap_or(true);
                if !owner.is_empty() && !process_exists {
                    drop(fs::remove_file(&path).await);
                }
                sleep(Duration::from_millis(10)).await;
            }
            Err(error) => panic!("could not create migration test lock: {error}"),
        }
    }
}

async fn lock_connection(config: Config) -> (PostgresClient, JoinSet<Result<(), PostgresError>>) {
    let (client, connection) = config.connect(NoTls).await.unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute("SELECT pg_advisory_lock(72707368::bigint)")
        .await
        .unwrap();
    (client, tasks)
}

async fn release_lock(client: PostgresClient, tasks: &mut JoinSet<Result<(), PostgresError>>) {
    client
        .batch_execute("SELECT pg_advisory_unlock(72707368::bigint)")
        .await
        .unwrap();
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
}

async fn clear_deployment_lock(pool: &Pool) {
    let result = timeout(Duration::from_secs(5), async {
        loop {
            let client = pool.get().await.unwrap();
            let locks: i64 = client
                .query_one(
                    "SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND objid=72707369",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            if locks == 0 {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    result.unwrap();
}

async fn context(pool: Pool) -> Database {
    let client = pool.get().await.unwrap();
    let schema = client
        .query_one("SELECT current_schema()", &[])
        .await
        .unwrap()
        .get(0);
    client
        .batch_execute(include_str!("fixtures/ledger.sql"))
        .await
        .unwrap();
    drop(client);
    Database { pool, schema }
}

impl Database {
    pub(super) async fn run(&self, scripts: &[(&str, &str)]) -> Result<(), DeployError> {
        let client = self.pool.get().await.unwrap();
        let deadline = Instant::now().checked_add(Duration::from_secs(10)).unwrap();
        let service = self.service(scripts);
        service.run(client, deadline).await
    }

    pub(super) fn service(&self, scripts: &[(&str, &str)]) -> MigrationDeploy {
        let catalog = scripts
            .iter()
            .map(|(name, sql)| Migration {
                name: (*name).to_owned(),
                checksum: hex::encode(Sha256::digest(sql.as_bytes())),
                sql: sql.as_bytes().to_vec(),
            })
            .collect();
        MigrationDeploy {
            schema: self.schema.clone(),
            catalog,
            next_id: Uuid::new_v4,
        }
    }

    pub(super) async fn sql(&self, sql: &str) {
        let client = self.pool.get().await.unwrap();
        client.batch_execute(sql).await.unwrap();
    }

    pub(super) async fn rows(&self) -> Vec<Row> {
        let client = self.pool.get().await.unwrap();
        client.query("SELECT id, migration_name, checksum, applied_steps_count, logs, finished_at IS NOT NULL AS finished, rolled_back_at IS NOT NULL AS rolled_back, started_at IS NOT NULL AS started FROM _prisma_migrations ORDER BY started_at, id", &[]).await.unwrap()
    }

    pub(super) async fn count(&self, sql: &str) -> i64 {
        let client = self.pool.get().await.unwrap();
        client.query_one(sql, &[]).await.unwrap().get(0)
    }

    pub(super) async fn wait_for(&self, sql: &str) {
        let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
        while self.count(sql).await == 0 {
            assert!(Instant::now() < deadline, "checkpoint was not reached");
            sleep(Duration::from_millis(10)).await;
        }
    }

    pub(super) async fn lock_conflict(&self, lock_sql: &str) {
        let blocker = self.pool.get().await.unwrap();
        blocker.batch_execute(lock_sql).await.unwrap();
        let client = self.pool.get().await.unwrap();
        let service = self.service(&[("locked", "SELECT 1")]);
        let mut deployment = Box::pin(service.run(
            client,
            Instant::now().checked_add(Duration::from_secs(10)).unwrap(),
        ));
        checkpoint(deployment.as_mut(), sleep(Duration::from_millis(100))).await;
        assert!(self.rows().await.is_empty());
        blocker
            .batch_execute("COMMIT; SELECT pg_advisory_unlock_all()")
            .await
            .unwrap();
        deployment.await.unwrap();
        assert!(self.rows().await[0].get::<_, bool>("finished"));
    }

    pub(super) async fn columns(&self) -> Vec<String> {
        let client = self.pool.get().await.unwrap();
        client.query("SELECT column_name || ':' || data_type || ':' || is_nullable || ':' || coalesce(column_default, '') || ':' || coalesce(character_maximum_length::text, '') FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = '_prisma_migrations' ORDER BY ordinal_position", &[]).await.unwrap().iter().map(|row| row.get(0)).collect()
    }
}

use sha2::{Digest as _, Sha256};

pub(super) struct CatalogDirectory(pub PathBuf);

impl CatalogDirectory {
    pub(super) async fn create() -> Self {
        let root = temp_dir().join(Uuid::new_v4().to_string());
        fs::create_dir_all(root.join("migrations")).await.unwrap();
        Self(root)
    }

    pub(super) async fn write(&self, path: &str, bytes: &[u8]) {
        let destination = self.0.join(path);
        fs::create_dir_all(destination.parent().unwrap())
            .await
            .unwrap();
        fs::write(destination, bytes).await.unwrap();
    }
}

impl Drop for CatalogDirectory {
    fn drop(&mut self) {
        remove_dir_all(&self.0).unwrap();
    }
}

pub(super) async fn checkpoint<F, G>(mut deployment: Pin<&mut F>, waiting: G)
where
    F: Future<Output = Result<(), DeployError>>,
    G: Future<Output = ()>,
{
    let mut observation = Box::pin(waiting);
    poll_fn(|context| {
        assert!(
            deployment.as_mut().poll(context).is_pending(),
            "deployment completed before the checkpoint"
        );
        observation.as_mut().poll(context)
    })
    .await;
}
