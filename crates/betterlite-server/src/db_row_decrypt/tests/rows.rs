use std::sync::atomic::Ordering;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::support::capture_logs;
use crate::db_row_decrypt::rows::read_model_rows;

const SCHEMA: &str = r#"
    CREATE TABLE "LiteLLM_ProxyModelTable" (
        model_id text PRIMARY KEY,
        model_name text NOT NULL,
        litellm_params jsonb NOT NULL,
        model_info jsonb
    );
"#;

const UNRELATED_SCHEMA: &str = r#"CREATE TABLE "Unrelated" (id text PRIMARY KEY);"#;

const INSERT_ROW: &str = "INSERT INTO \"LiteLLM_ProxyModelTable\" \
    (model_id, model_name, litellm_params) VALUES ($1, 'gpt', $2::text::jsonb)";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Database {
    fixture: PostgresFixture,
    pool: Pool,
}

impl Database {
    async fn start(schema: &str) -> Self {
        let url = Environment::parse_from(["db-row-decrypt-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![schema.to_owned()])
                .resolve()
                .await
                .expect("a Postgres database fixture");
        let config = fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses");
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(2)
            .build()
            .expect("the fixture pool builds");
        Self { fixture, pool }
    }

    fn store(&self) -> PostgresStore {
        PostgresStore {
            pool: Some(self.pool.clone()),
        }
    }

    async fn insert(&self, model_id: &str, litellm_params: &str) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(INSERT_ROW, &[&model_id, &litellm_params])
            .await
            .expect("the row inserts");
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

fn disconnected() -> PostgresStore {
    PostgresStore { pool: None }
}

// WriterPinnedClient: a healthy writer serves the read even when the configured reader is down.
#[tokio::test]
async fn reads_through_the_writer_while_the_reader_is_down() {
    let database = Database::start(SCHEMA).await;
    database
        .insert("dep-1", r#"{"model":"openai/gpt-4o"}"#)
        .await;
    let writer = database.store();
    let reader = disconnected();
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &reader,
        availability: &availability,
    };
    let rows = read_model_rows(Some(client)).await.expect("the rows read");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields.model_id, "dep-1");
    database.close().await;
}

// A degraded writer leaves the read on the replica, which is where LiteLLM's WriterPinnedClient sends it.
#[tokio::test]
async fn a_degraded_writer_reads_through_the_replica() {
    let database = Database::start(SCHEMA).await;
    database
        .insert("dep-1", r#"{"model":"openai/gpt-4o"}"#)
        .await;
    let writer = disconnected();
    let reader = database.store();
    let availability = Availability::default();
    availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    let client = PrismaClient {
        writer: &writer,
        reader: &reader,
        availability: &availability,
    };
    let rows = read_model_rows(Some(client)).await.expect("the rows read");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields.model_id, "dep-1");
    database.close().await;
}

#[tokio::test]
async fn a_missing_client_returns_none_and_logs() {
    let (logs, _guard) = capture_logs();
    assert!(read_model_rows(None).await.is_none());
    assert!(
        logs.contents()
            .contains("Error getting new models from DB - no database client")
    );
}

#[tokio::test]
async fn a_failed_read_returns_none_and_logs() {
    let (logs, _guard) = capture_logs();
    let database = Database::start(UNRELATED_SCHEMA).await;
    let writer = database.store();
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    assert!(read_model_rows(Some(client)).await.is_none());
    assert!(
        logs.contents()
            .contains("Error getting new models from DB - ")
    );
    database.close().await;
}

#[tokio::test]
async fn an_undecodable_row_returns_none() {
    let (logs, _guard) = capture_logs();
    let database = Database::start(SCHEMA).await;
    database.insert("dep-1", r#""not-json""#).await;
    let writer = database.store();
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    assert!(read_model_rows(Some(client)).await.is_none());
    assert!(
        logs.contents()
            .contains("Error getting new models from DB - ")
    );
    database.close().await;
}
