use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::health_check_rows::{HealthCheckIdSource, HealthCheckRows};
use crate::postgres_store::session::PostgresStore;
use crate::schema_daily_usage::ADD_HEALTH_CHECK_TABLE;
use crate::schema_policy_resources::HEALTH_CHECK_LATEST_PER_MODEL;

const SCHEMA: &str = "health_check_rows_tests";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn configured_url() -> Option<SecretString> {
    Environment::parse_from(["health-check-rows-tests"])
        .url
        .map(SecretString::from)
}

fn pool(url: &SecretString) -> Pool {
    let mut config: tokio_postgres::Config = url.expose_secret().parse().unwrap();
    config.options(format!("-csearch_path={SCHEMA}"));
    let manager = Manager::new(config, NoTls);
    Pool::builder(manager).max_size(4).build().unwrap()
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
    pub(crate) client: Client,
}

pub(crate) async fn start() -> Database {
    // B703's table and B706's per-model index, in a schema of this test's own inside the fixture's database
    let statements = vec![
        format!("CREATE SCHEMA IF NOT EXISTS \"{SCHEMA}\""),
        format!("SET search_path TO \"{SCHEMA}\""),
        str::from_utf8(ADD_HEALTH_CHECK_TABLE.bytes)
            .unwrap()
            .to_owned(),
        str::from_utf8(HEALTH_CHECK_LATEST_PER_MODEL.bytes)
            .unwrap()
            .to_owned(),
    ];
    let fixture = PostgresFixture::provision_with_statements(configured_url().as_ref(), statements)
        .resolve()
        .await
        .unwrap();
    let store = PostgresStore {
        pool: Some(pool(&fixture.options.url)),
    };
    let client = store.session().await.unwrap();
    Database {
        fixture,
        store,
        client,
    }
}

impl Database {
    pub(crate) fn rows(&self) -> HealthCheckRows {
        HealthCheckRows {
            store: self.store.clone(),
            new_id: Arc::new(|| uuid::Uuid::new_v4().to_string()),
        }
    }

    pub(crate) fn rows_with_id(&self, id: &'static str) -> HealthCheckRows {
        HealthCheckRows {
            store: self.store.clone(),
            new_id: Arc::new(move || id.to_owned()),
        }
    }

    pub(crate) async fn finish(self) {
        drop(self.client);
        if let Some(pool) = self.store.pool {
            pool.close();
        }
        self.fixture.close_async().await.unwrap();
    }
}

// a store with no pool: every call reaches the driver's "No DB Connected" error without a database
pub(crate) fn unconnected() -> HealthCheckRows {
    let source: HealthCheckIdSource = Arc::new(|| String::from("unused-id"));
    HealthCheckRows {
        store: PostgresStore { pool: None },
        new_id: source,
    }
}

#[derive(Clone, Default)]
pub(crate) struct Seed {
    pub(crate) id: &'static str,
    pub(crate) model_name: &'static str,
    pub(crate) model_id: Option<&'static str>,
    pub(crate) status: &'static str,
    pub(crate) checked_at: &'static str,
    pub(crate) details: Option<&'static str>,
}

impl Seed {
    pub(crate) const fn new(
        id: &'static str,
        model_name: &'static str,
        checked_at: &'static str,
    ) -> Self {
        Self {
            id,
            model_name,
            model_id: None,
            status: "healthy",
            checked_at,
            details: None,
        }
    }
}

pub(crate) async fn insert_seed(client: &Client, seed: &Seed) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_HealthCheckTable"
                ("health_check_id", "model_name", "model_id", "status", "healthy_count",
                 "unhealthy_count", "details", "checked_at", "created_at", "updated_at")
                VALUES ($1, $2, $3, $4, 2, 0, $5::jsonb, $6::timestamp, $6::timestamp, $6::timestamp)"#,
            &[
                &seed.id,
                &seed.model_name,
                &seed.model_id,
                &seed.status,
                &seed.details,
                &seed.checked_at,
            ],
        )
        .await
        .unwrap();
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (Logs, DefaultGuard) {
    let buffer = Logs::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
