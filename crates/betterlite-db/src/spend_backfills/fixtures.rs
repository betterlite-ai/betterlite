#![cfg(test)]

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{NaiveDate, NaiveDateTime};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, Config, NoTls};

use crate::postgres_store::session::PostgresStore;

pub const SCHEMA_SQL: &str = include_str!("fixtures/schema.sql");

const LOG_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, "startTime") VALUES ($1, $2, $3, $4, $5)"#;
const TOOL_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogToolIndex" (request_id, tool_name, start_time) VALUES ($1, $2, $3)"#;
const ACTIVE_KEY_SQL: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend, total_spend, budget_duration, budget_id) VALUES ($1, $2, $3, $4, $5)"#;
const ARCHIVED_KEY_SQL: &str = r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token, spend, total_spend, budget_duration, budget_id) VALUES ($1, $2, $3, $4, $5, $6)"#;
const BUDGET_SQL: &str =
    r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, budget_duration) VALUES ($1, $2)"#;
const BUCKETS_SQL: &str = r#"SELECT date, tool_name, spend, total_tokens, request_count FROM "LiteLLM_DailyToolSpend" ORDER BY date, tool_name"#;
const ACTIVE_ROW_SQL: &str =
    r#"SELECT spend, total_spend FROM "LiteLLM_VerificationToken" WHERE token = $1"#;
const ARCHIVED_ROW_SQL: &str =
    r#"SELECT spend, total_spend FROM "LiteLLM_DeletedVerificationToken" WHERE id = $1"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub struct SpendLogSeed<'seed> {
    pub request_id: &'seed str,
    pub api_key: &'seed str,
    pub spend: Decimal,
    pub total_tokens: i32,
    pub start_time: NaiveDateTime,
}

#[derive(Clone, Copy)]
pub enum KeyTable {
    Active,
    Archived,
}

#[derive(Clone, Copy)]
pub struct KeySeed<'seed> {
    pub token: &'seed str,
    pub spend: Decimal,
    pub total_spend: Decimal,
    pub budget_duration: Option<&'seed str>,
    pub budget_id: Option<&'seed str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyBucket {
    pub date: String,
    pub tool_name: String,
    pub spend: Decimal,
    pub total_tokens: i64,
    pub request_count: i64,
}

pub struct SpendBackfillCase {
    fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
    pub(crate) client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl SpendBackfillCase {
    pub async fn new(session_time_zone: Option<&str>) -> Self {
        let url = Environment::parse_from(["spend-backfills"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .expect("a test schema");
        let store = store(&fixture, session_time_zone);
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        client
            .batch_execute(SCHEMA_SQL)
            .await
            .expect("the pinned tables");
        Self {
            fixture,
            store,
            client,
            tasks,
        }
    }

    pub async fn close(self) {
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

    pub async fn seed_log(&self, seed: SpendLogSeed<'_>) {
        let spend = seed.spend.to_f64().expect("a finite spend");
        self.client
            .execute(
                LOG_SQL,
                &[
                    &seed.request_id,
                    &seed.api_key,
                    &spend,
                    &seed.total_tokens,
                    &seed.start_time,
                ],
            )
            .await
            .expect("a spend log row");
    }

    pub async fn seed_tool(&self, request_id: &str, tool_name: &str, start_time: NaiveDateTime) {
        self.client
            .execute(TOOL_SQL, &[&request_id, &tool_name, &start_time])
            .await
            .expect("a tool index row");
    }

    pub async fn seed_key(&self, table: KeyTable, id: &str, seed: KeySeed<'_>) {
        let spend = seed.spend.to_f64().expect("a finite spend");
        let total_spend = seed.total_spend.to_f64().expect("a finite total spend");
        match table {
            KeyTable::Active => self
                .client
                .execute(
                    ACTIVE_KEY_SQL,
                    &[
                        &seed.token,
                        &spend,
                        &total_spend,
                        &seed.budget_duration,
                        &seed.budget_id,
                    ],
                )
                .await
                .expect("an active key row"),
            KeyTable::Archived => self
                .client
                .execute(
                    ARCHIVED_KEY_SQL,
                    &[
                        &id,
                        &seed.token,
                        &spend,
                        &total_spend,
                        &seed.budget_duration,
                        &seed.budget_id,
                    ],
                )
                .await
                .expect("an archived key row"),
        };
    }

    pub async fn seed_budget(&self, budget_id: &str, budget_duration: Option<&str>) {
        self.client
            .execute(BUDGET_SQL, &[&budget_id, &budget_duration])
            .await
            .expect("a budget row");
    }

    pub async fn daily_buckets(&self) -> Vec<DailyBucket> {
        let rows = self
            .client
            .query(BUCKETS_SQL, &[])
            .await
            .expect("the daily tool spend buckets");
        rows.iter()
            .map(|row| {
                let spend: f64 = row.get("spend");
                DailyBucket {
                    date: row.get("date"),
                    tool_name: row.get("tool_name"),
                    spend: Decimal::try_from(spend).expect("a finite spend"),
                    total_tokens: row.get("total_tokens"),
                    request_count: row.get("request_count"),
                }
            })
            .collect()
    }

    pub async fn key_row(&self, table: KeyTable, id: &str) -> (Option<Decimal>, Option<Decimal>) {
        let sql = match table {
            KeyTable::Active => ACTIVE_ROW_SQL,
            KeyTable::Archived => ARCHIVED_ROW_SQL,
        };
        let row = self.client.query_one(sql, &[&id]).await.expect("a key row");
        let spend: Option<f64> = row.get("spend");
        let total_spend: Option<f64> = row.get("total_spend");
        let spend_amount = spend.map(|amount| Decimal::try_from(amount).expect("a finite amount"));
        let total = total_spend.map(|amount| Decimal::try_from(amount).expect("a finite amount"));
        (spend_amount, total)
    }
}

// parse the URL: options.config re-encodes the schema option's leading space as '+', which Postgres rejects
fn store(fixture: &PostgresFixture, session_time_zone: Option<&str>) -> PostgresStore {
    let mut configuration: Config = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    let previous = configuration.get_options().unwrap_or_default().to_owned();
    if let Some(zone) = session_time_zone {
        configuration.options(format!("{previous} -ctimezone={zone}"));
    }
    let manager = Manager::new(configuration, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("a connection pool");
    PostgresStore { pool: Some(pool) }
}

#[must_use]
pub fn at(month: u32, day: u32, hour: u32, minute: u32, second: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, second))
        .expect("a valid test time")
}
