use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, Config, NoTls};

use crate::postgres_store::session::PostgresStore;

const TOOL_INDEX_SQL: &str =
    include_str!("../../../migrations/20260226120000_add_spend_log_tool_index/migration.sql");
const DAILY_TOOL_SPEND_SQL: &str =
    include_str!("../../../migrations/20260725000000_add_daily_tool_spend/migration.sql");
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
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .expect("a test schema");
        let store = store(&fixture);
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        client
            .batch_execute(TOOL_INDEX_SQL)
            .await
            .expect("the tool index table");
        client
            .batch_execute(DAILY_TOOL_SPEND_SQL)
            .await
            .expect("the daily tool spend table");
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
        fixture.close_async().await.expect("the schema is dropped");
    }

    pub(super) async fn execute(&self, sql: &str) {
        self.client
            .batch_execute(sql)
            .await
            .expect("a fixture statement");
    }

    pub(super) async fn index_rows(&self) -> Vec<(String, String, NaiveDateTime)> {
        let rows = self
            .client
            .query(INDEX_ROWS_SQL, &[])
            .await
            .expect("the tool index rows");
        rows.iter()
            .map(|row| {
                (
                    row.get("request_id"),
                    row.get("tool_name"),
                    row.get("start_time"),
                )
            })
            .collect()
    }

    pub(super) async fn daily_rollups(&self) -> Vec<(String, String, f64, i64, i64)> {
        let rows = self
            .client
            .query(DAILY_ROLLUPS_SQL, &[])
            .await
            .expect("the daily tool spend rollups");
        rows.iter()
            .map(|row| {
                (
                    row.get("date"),
                    row.get("tool_name"),
                    row.get("spend"),
                    row.get("total_tokens"),
                    row.get("request_count"),
                )
            })
            .collect()
    }
}

// parse the URL: options.config re-encodes the schema option's leading space as '+', which Postgres rejects
fn store(fixture: &PostgresFixture) -> PostgresStore {
    let configuration: Config = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    let manager = Manager::new(configuration, NoTls);
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
