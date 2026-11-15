#![cfg(test)]

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

use super::{SpendPartitionError, SpendPartitionOperation, run_conversion};

pub const SCHEMA_SQL: &str = include_str!("fixtures/schema.sql");

const LOG_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime") VALUES ($1, 'completion', 'sk-fixture', $2, $2)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub struct SpendPartitionCase {
    pub(crate) fixture: PostgresFixture,
    pub(crate) client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl SpendPartitionCase {
    pub async fn provision() -> Self {
        let url = Environment::parse_from(["spend-partitions"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .expect("a test schema");
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        client
            .batch_execute(SCHEMA_SQL)
            .await
            .expect("the pinned LiteLLM_SpendLogs schema");
        Self {
            fixture,
            client,
            tasks,
        }
    }

    pub async fn close(self) {
        let Self {
            fixture,
            client,
            tasks,
        } = self;
        drop(client);
        drop(tasks);
        fixture.close_async().await.expect("the schema is dropped");
    }

    pub async fn run_ok(&self, operation: SpendPartitionOperation) {
        run_conversion(&self.client, operation)
            .await
            .expect("the spend partition conversion");
    }

    #[must_use]
    pub async fn run_error(&self, operation: SpendPartitionOperation) -> SpendPartitionError {
        run_conversion(&self.client, operation)
            .await
            .expect_err("a failing spend partition conversion")
    }

    pub async fn seed_log(&self, request_id: &str, start_time: NaiveDateTime) {
        self.client
            .execute(LOG_SQL, &[&request_id, &start_time])
            .await
            .expect("a spend log row");
    }
}
