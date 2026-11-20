use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, Config, NoTls};

use super::support::{FixedJitter, RecordingPause, key, transaction};
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::model_usage::writer::{ModelUsageWriteError, PostgresModelUsageWriter};
use crate::model_usage::{
    ModelUsageFlushError, ModelUsageTransaction, flush_model_usage_transactions,
};
use crate::postgres_store::session::PostgresStore;
use crate::tool_spend::DEFAULT_RETRIES;

const ROLLUPS_SQL: &str = r#"SELECT date, model_group, model, custom_llm_provider, task_type, spend::numeric AS spend, prompt_tokens, completion_tokens, request_count, successful_requests, failed_requests FROM "LiteLLM_DailyModelUsage" ORDER BY date, model_group, model, custom_llm_provider, task_type"#;

const REJECT_BLOCKED_SQL: &str = r#"CREATE FUNCTION reject_blocked_model() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.model = 'blocked' THEN RAISE EXCEPTION 'blocked model'; END IF; RETURN NEW; END $$;
CREATE TRIGGER reject_blocked_model BEFORE INSERT OR UPDATE ON "LiteLLM_DailyModelUsage" FOR EACH ROW EXECUTE FUNCTION reject_blocked_model();"#;

const SEED_ALPHA_SQL: &str = r#"INSERT INTO "LiteLLM_DailyModelUsage" (date, model_group, model, custom_llm_provider, task_type, spend, prompt_tokens, completion_tokens, request_count, successful_requests, failed_requests, updated_at) VALUES ('2026-01-02', 'group', 'alpha', 'openai', 'uncategorized', 1.0, 10, 2, 1, 1, 0, now())"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

#[derive(Debug, PartialEq)]
struct RollupRow {
    date: String,
    model_group: String,
    model: String,
    custom_llm_provider: String,
    task_type: String,
    spend: Decimal,
    prompt_tokens: i64,
    completion_tokens: i64,
    request_count: i64,
    successful_requests: i64,
    failed_requests: i64,
}

fn usage_row(
    model: &str,
    spend: Decimal,
    tokens: (i64, i64),
    counts: (i64, i64, i64),
) -> RollupRow {
    RollupRow {
        date: String::from("2026-01-02"),
        model_group: String::from("group"),
        model: String::from(model),
        custom_llm_provider: String::from("openai"),
        task_type: String::from("uncategorized"),
        spend,
        prompt_tokens: tokens.0,
        completion_tokens: tokens.1,
        request_count: counts.0,
        successful_requests: counts.1,
        failed_requests: counts.2,
    }
}

struct ModelUsageCase {
    fixture: PostgresFixture,
    store: PostgresStore,
    client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl ModelUsageCase {
    async fn new() -> Self {
        let url = Environment::parse_from(["model-usage"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            url.as_ref(),
            catalog_statements(&PACKAGED_CATALOGS).expect("the packaged catalogs are UTF-8"),
        )
        .resolve()
        .await
        .expect("a test database");
        let store = pool_store(fixture.options.config.clone());
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

    async fn close(self) {
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

    async fn execute(&self, sql: &str) {
        self.client
            .batch_execute(sql)
            .await
            .expect("a fixture statement");
    }

    async fn rollups(&self) -> Vec<RollupRow> {
        let rows = self
            .client
            .query(ROLLUPS_SQL, &[])
            .await
            .expect("a fixture query");
        rows.iter()
            .map(|row| RollupRow {
                date: row.get("date"),
                model_group: row.get("model_group"),
                model: row.get("model"),
                custom_llm_provider: row.get("custom_llm_provider"),
                task_type: row.get("task_type"),
                spend: row.get("spend"),
                prompt_tokens: row.get("prompt_tokens"),
                completion_tokens: row.get("completion_tokens"),
                request_count: row.get("request_count"),
                successful_requests: row.get("successful_requests"),
                failed_requests: row.get("failed_requests"),
            })
            .collect()
    }
}

fn pool_store(configuration: Config) -> PostgresStore {
    let manager = Manager::new(configuration, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("a connection pool");
    PostgresStore { pool: Some(pool) }
}

fn unreachable_store() -> PostgresStore {
    pool_store(
        "postgres://spend@127.0.0.1:1/spend"
            .parse()
            .expect("a URL no server listens on"),
    )
}

fn sample(
    model: &str,
    spend: Decimal,
    tokens: (i64, i64),
    successful: bool,
) -> ModelUsageTransaction {
    transaction(
        key("2026-01-02", "group", model, "openai", "uncategorized"),
        spend,
        tokens,
        successful,
    )
}

async fn flush(
    store: &PostgresStore,
    transactions: &[ModelUsageTransaction],
) -> Result<(), ModelUsageFlushError<ModelUsageWriteError>> {
    let writer = PostgresModelUsageWriter {
        store: store.clone(),
    };
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let outcome =
        flush_model_usage_transactions(&writer, transactions, DEFAULT_RETRIES, &jitter, &pause)
            .await;
    assert!(pause.delays().is_empty());
    outcome
}

#[tokio::test]
async fn the_writer_adds_each_flush_to_the_existing_rows() {
    let case = ModelUsageCase::new().await;
    let transactions = [
        sample("alpha", Decimal::new(25, 2), (10, 5), true),
        sample("alpha", Decimal::new(25, 2), (10, 5), true),
        sample("beta", Decimal::new(5, 1), (3, 1), false),
    ];
    flush(&case.store, &transactions)
        .await
        .expect("the first flush writes");
    flush(&case.store, &transactions)
        .await
        .expect("the second flush adds");
    assert_eq!(
        case.rollups().await,
        [
            usage_row("alpha", Decimal::new(100, 2), (40, 20), (4, 4, 0)),
            usage_row("beta", Decimal::new(100, 2), (6, 2), (2, 0, 2)),
        ]
    );
    case.close().await;
}

#[tokio::test]
async fn a_failed_statement_rolls_back_every_increment_of_the_flush() {
    let case = ModelUsageCase::new().await;
    case.execute(REJECT_BLOCKED_SQL).await;
    case.execute(SEED_ALPHA_SQL).await;
    let transactions = [
        sample("alpha", Decimal::new(25, 2), (10, 5), true),
        transaction(
            key("2026-01-02", "group", "blocked", "openai", "uncategorized"),
            Decimal::new(5, 1),
            (3, 1),
            true,
        ),
    ];
    let writer = PostgresModelUsageWriter {
        store: case.store.clone(),
    };
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let outcome = flush_model_usage_transactions(&writer, &transactions, 1, &jitter, &pause).await;
    assert!(matches!(
        outcome,
        Err(ModelUsageFlushError::Write(
            ModelUsageWriteError::AfterSend(_)
        ))
    ));
    assert!(pause.delays().is_empty());
    assert_eq!(
        case.rollups().await,
        [usage_row("alpha", Decimal::new(100, 2), (10, 2), (1, 1, 0))]
    );
    case.close().await;
}

#[tokio::test]
async fn an_unreachable_pool_is_retried_and_then_reported() {
    let writer = PostgresModelUsageWriter {
        store: unreachable_store(),
    };
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let outcome = flush_model_usage_transactions(
        &writer,
        &[sample("alpha", Decimal::new(25, 2), (10, 5), true)],
        1,
        &jitter,
        &pause,
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ModelUsageFlushError::Write(ModelUsageWriteError::Connect(
            _
        )))
    ));
    assert_eq!(pause.delays(), [Duration::from_secs(1)]);
}

#[tokio::test]
async fn an_empty_flush_never_reaches_the_database() {
    let writer = PostgresModelUsageWriter {
        store: unreachable_store(),
    };
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    flush_model_usage_transactions(&writer, &[], DEFAULT_RETRIES, &jitter, &pause)
        .await
        .expect("nothing to write");
    assert!(pause.delays().is_empty());
}
