use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, Config, NoTls, Row};

use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::postgres_store::session::PostgresStore;

const INDEX_ROWS_SQL: &str = r#"SELECT request_id, tool_name, start_time FROM "LiteLLM_SpendLogToolIndex" ORDER BY request_id, tool_name"#;
const DAILY_ROLLUPS_SQL: &str = r#"SELECT date, tool_name, spend, total_tokens, request_count FROM "LiteLLM_DailyToolSpend" ORDER BY date, tool_name"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct ToolSpendCase {
    fixture: PostgresFixture,
    pub(super) store: PostgresStore,
    client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl ToolSpendCase {
    pub(super) async fn new() -> Self {
        let url = Environment::parse_from(["tool-spend"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            url.as_ref(),
            catalog_statements(&PACKAGED_CATALOGS).unwrap(),
        )
        .resolve()
        .await
        .expect("a test database");
        let store = store(&fixture);
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        Self {
            fixture,
            store,
            client,
            tasks,
        }
    }

    pub(super) async fn close(self) {
        let Self {
            fixture,
            store,
            client,
            tasks,
        } = self;
        drop(client);
        drop(tasks);
        if let Some(pool) = store.pool {
            pool.close();
        }
        fixture
            .close_async()
            .await
            .expect("the database is dropped");
    }

    pub(super) async fn execute(&self, sql: &str) {
        self.client
            .batch_execute(sql)
            .await
            .expect("a fixture statement");
    }

    pub(super) async fn index_rows(&self) -> Vec<(String, String, NaiveDateTime)> {
        self.rows(INDEX_ROWS_SQL, |row| {
            (
                row.get("request_id"),
                row.get("tool_name"),
                row.get("start_time"),
            )
        })
        .await
    }

    pub(super) async fn daily_rollups(&self) -> Vec<(String, String, f64, i64, i64)> {
        self.rows(DAILY_ROLLUPS_SQL, |row| {
            (
                row.get("date"),
                row.get("tool_name"),
                row.get("spend"),
                row.get("total_tokens"),
                row.get("request_count"),
            )
        })
        .await
    }

    async fn rows<T, Read>(&self, sql: &str, read: Read) -> Vec<T>
    where
        Read: Fn(&Row) -> T,
    {
        let rows = self.client.query(sql, &[]).await.expect("a fixture query");
        rows.iter().map(read).collect()
    }
}

fn store(fixture: &PostgresFixture) -> PostgresStore {
    let manager = Manager::new(fixture.options.config.clone(), NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("a connection pool");
    PostgresStore { pool: Some(pool) }
}

pub(super) fn unreachable_store() -> PostgresStore {
    let configuration: Config = "postgres://spend@127.0.0.1:1/spend"
        .parse()
        .expect("a URL no server listens on");
    let manager = Manager::new(configuration, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("a connection pool");
    PostgresStore { pool: Some(pool) }
}
