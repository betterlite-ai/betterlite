use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Barrier;
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::postgres_store::session::PostgresStore;
use crate::spend_queues::entity::SpendAmount;
use crate::window_queue::WindowSpendTransaction;
use crate::window_spend::error::WindowSpendError;
use crate::window_spend::seed::{SeedQuery, WindowSeedTotals, WindowSpendLogs};
use crate::window_spend::store::{WindowRoll, WindowSpendStore};

const TABLES: &str = r#"
CREATE TABLE "LiteLLM_BudgetWindowSpend" (
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    window_duration TEXT NOT NULL,
    window_start TIMESTAMP(3) NOT NULL,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP(3) NOT NULL,
    PRIMARY KEY (entity_type, entity_id, window_duration)
);
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT,
    api_key TEXT,
    team_id TEXT,
    spend DOUBLE PRECISION,
    "startTime" TIMESTAMP(3) NOT NULL
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal amount")
}

pub(crate) fn naive(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").expect("a naive moment")
}

pub(crate) fn utc(text: &str) -> DateTime<Utc> {
    naive(text).and_utc()
}

pub(crate) fn totals(total: &str, before_batch: &str) -> WindowSeedTotals {
    WindowSeedTotals {
        total: decimal(total),
        before_batch: decimal(before_batch),
    }
}

pub(crate) struct Increment<'text> {
    pub(crate) entity_type: &'text str,
    pub(crate) entity_id: &'text str,
    pub(crate) window_duration: &'text str,
    pub(crate) window_start: &'text str,
    pub(crate) spend: &'text str,
    pub(crate) started_at: Option<&'text str>,
}

pub(crate) fn transaction(increment: &Increment<'_>) -> WindowSpendTransaction {
    WindowSpendTransaction {
        entity_type: increment.entity_type.to_owned(),
        entity_id: increment.entity_id.to_owned(),
        window_duration: increment.window_duration.to_owned(),
        window_start: increment.window_start.to_owned(),
        spend: SpendAmount(decimal(increment.spend)),
        started_at: increment.started_at.map(str::to_owned),
    }
}

pub(crate) fn window_transaction(
    entity_type: &str,
    entity_id: &str,
    window_duration: &str,
    window_start: &str,
    spend: &str,
) -> WindowSpendTransaction {
    transaction(&Increment {
        entity_type,
        entity_id,
        window_duration,
        window_start,
        spend,
        started_at: None,
    })
}

pub(crate) fn key_transaction(
    entity_id: &str,
    window_start: &str,
    spend: &str,
) -> WindowSpendTransaction {
    window_transaction("key", entity_id, "1h", window_start, spend)
}

pub(crate) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["window-spend-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![TABLES.to_owned()])
        .resolve()
        .await
        .expect("the database fixture");
    let manager = Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    );
    let pool = Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("the database pool");
    (fixture, pool)
}

pub(crate) fn store(pool: &Pool) -> WindowSpendStore {
    WindowSpendStore::from_store(PostgresStore {
        pool: Some(pool.clone()),
    })
}

pub(crate) fn log_store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

pub(crate) async fn execute(pool: &Pool, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
    let client = pool.get().await.expect("the database connection");
    client
        .execute(statement, parameters)
        .await
        .expect("the fixture row");
}

pub(crate) struct StoredWindow {
    pub(crate) window_start: NaiveDateTime,
    pub(crate) spend: Decimal,
}

pub(crate) async fn window_row(
    pool: &Pool,
    entity_type: &str,
    entity_id: &str,
    window_duration: &str,
) -> Option<StoredWindow> {
    let client = pool.get().await.expect("the database connection");
    let statement = r#"SELECT window_start, spend::numeric FROM "LiteLLM_BudgetWindowSpend" WHERE entity_type = $1 AND entity_id = $2 AND window_duration = $3"#;
    let row = client
        .query_opt(statement, &[&entity_type, &entity_id, &window_duration])
        .await
        .expect("the stored window");
    row.map(|found| StoredWindow {
        window_start: found.get(0),
        spend: found.get(1),
    })
}

pub(crate) async fn stored_window(
    pool: &Pool,
    entity_type: &str,
    entity_id: &str,
    window_duration: &str,
) -> StoredWindow {
    window_row(pool, entity_type, entity_id, window_duration)
        .await
        .expect("a stored window")
}

pub(crate) async fn window_timestamps(
    pool: &Pool,
    entity_type: &str,
    entity_id: &str,
    window_duration: &str,
) -> (NaiveDateTime, NaiveDateTime) {
    let client = pool.get().await.expect("the database connection");
    let statement = r#"SELECT created_at, updated_at FROM "LiteLLM_BudgetWindowSpend" WHERE entity_type = $1 AND entity_id = $2 AND window_duration = $3"#;
    let row = client
        .query_one(statement, &[&entity_type, &entity_id, &window_duration])
        .await
        .expect("the stored window timestamps");
    (row.get(0), row.get(1))
}

pub(crate) async fn insert_window(
    pool: &Pool,
    entity_type: &str,
    entity_id: &str,
    window_start: &str,
    spend: &str,
) {
    let statement = r#"INSERT INTO "LiteLLM_BudgetWindowSpend" (entity_type, entity_id, window_duration, window_start, spend, updated_at) VALUES ($1, $2, '1h', $3, $4::numeric::double precision, '2025-12-31T23:00:00.000')"#;
    execute(
        pool,
        statement,
        &[
            &entity_type,
            &entity_id,
            &naive(window_start),
            &decimal(spend),
        ],
    )
    .await;
}

pub(crate) async fn insert_log(
    pool: &Pool,
    api_key: &str,
    team_id: &str,
    spend: &str,
    start_time: &str,
) {
    let statement = r#"INSERT INTO "LiteLLM_SpendLogs" (api_key, team_id, spend, "startTime") VALUES ($1, $2, $3::numeric::double precision, $4)"#;
    execute(
        pool,
        statement,
        &[&api_key, &team_id, &decimal(spend), &naive(start_time)],
    )
    .await;
}

pub(crate) async fn roll_window(pool: &Pool, entity_id: &str, new_window_start: &str, now: &str) {
    store(pool)
        .roll_window_spend_row(&WindowRoll {
            entity_type: "key",
            entity_id,
            window_duration: "1h",
            new_window_start: naive(new_window_start),
            now: utc(now),
        })
        .await
        .expect("the rolled window");
}

pub(crate) async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture cleanup");
}

pub(crate) struct StaticLogs {
    pub(crate) totals: WindowSeedTotals,
}

impl WindowSpendLogs for StaticLogs {
    async fn seed_totals(
        &self,
        _query: &SeedQuery<'_>,
    ) -> Result<WindowSeedTotals, WindowSpendError> {
        Ok(self.totals)
    }
}

pub(crate) struct BarrierLogs {
    pub(crate) totals: WindowSeedTotals,
    pub(crate) barrier: Arc<Barrier>,
}

impl WindowSpendLogs for BarrierLogs {
    async fn seed_totals(
        &self,
        _query: &SeedQuery<'_>,
    ) -> Result<WindowSeedTotals, WindowSpendError> {
        let _released = self.barrier.wait().await;
        Ok(self.totals)
    }
}

pub(crate) struct CountingLogs {
    pub(crate) totals: WindowSeedTotals,
    pub(crate) calls: AtomicUsize,
}

impl CountingLogs {
    pub(crate) fn with_totals(totals: WindowSeedTotals) -> Self {
        Self {
            totals,
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl WindowSpendLogs for CountingLogs {
    async fn seed_totals(
        &self,
        _query: &SeedQuery<'_>,
    ) -> Result<WindowSeedTotals, WindowSpendError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.totals)
    }
}
