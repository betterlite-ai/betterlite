use super::super::support::{FakeReads, fixed_now, scope};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityScope, DailyActivityScopeInput};
use crate::daily_activity_repository::DailyActivityRepository;
use crate::daily_activity_repository::database::PostgresDailyActivityDatabase;
use crate::{
    schema_agent_identity, schema_baseline, schema_budget_windows, schema_daily_usage,
    schema_identity_history, schema_managed_resources, schema_policy_resources,
    schema_routing_state,
};
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use futures_util::FutureExt as _;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use secrecy::SecretString;
use std::future::Future;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::str;
use tokio_postgres::NoTls;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) type PostgresRepository<'client> = DailyActivityRepository<
    PostgresDailyActivityDatabase<'client>,
    FakeReads,
    fn() -> DateTime<Utc>,
>;

pub(super) async fn run<Body, Test>(body: Body)
where
    Body: FnOnce(Pool) -> Test,
    Test: Future<Output = ()>,
{
    with_databases(1, |mut pools| body(pools.remove(0))).await;
}

pub(super) async fn run_pair<Body, Test>(body: Body)
where
    Body: FnOnce(Pool, Pool) -> Test,
    Test: Future<Output = ()>,
{
    with_databases(2, |mut pools| {
        let second = pools.remove(1);
        body(pools.remove(0), second)
    })
    .await;
}

async fn with_databases<Body, Test>(count: usize, body: Body)
where
    Body: FnOnce(Vec<Pool>) -> Test,
    Test: Future<Output = ()>,
{
    let url = Environment::parse_from(["daily-activity-repository"])
        .url
        .map(SecretString::from);
    let mut resources: Vec<(PostgresFixture, Pool)> = Vec::new();
    let outcome = AssertUnwindSafe(async {
        for _ in 0..count {
            let mut task = PostgresFixture::provision_with_name_generator(
                url.as_ref(),
                &FixtureSeed::default(),
                &mut RandomDatabaseName,
            );
            let fixture = task.resolve().await.unwrap();
            let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
                .max_size(6)
                .build()
                .unwrap();
            migrate(&pool).await;
            resources.push((fixture, pool));
        }
        body(resources.iter().map(|(_, pool)| pool.clone()).collect()).await;
    })
    .catch_unwind()
    .await;
    for (fixture, pool) in resources {
        pool.close();
        fixture.close_async().await.unwrap();
    }
    if let Err(payload) = outcome {
        resume_unwind(payload);
    }
}

async fn migrate(pool: &Pool) {
    let client = pool.get().await.unwrap();
    client
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .await
        .unwrap();
    for catalog in [
        schema_baseline::MIGRATION_CATALOG,
        schema_daily_usage::MIGRATION_CATALOG,
        schema_managed_resources::MIGRATION_CATALOG,
        schema_identity_history::MIGRATION_CATALOG,
        schema_policy_resources::MIGRATION_CATALOG,
        schema_routing_state::MIGRATION_CATALOG,
        schema_budget_windows::MIGRATION_CATALOG,
        schema_agent_identity::MIGRATION_CATALOG,
    ] {
        for migration in catalog.files {
            client
                .batch_execute(str::from_utf8(migration.bytes).unwrap())
                .await
                .unwrap();
        }
    }
}

pub(super) fn repository<'client>(
    writer: &'client Client,
    reader: Option<&'client Client>,
) -> PostgresRepository<'client> {
    DailyActivityRepository {
        database: PostgresDailyActivityDatabase { writer, reader },
        proxy_reads: FakeReads::default(),
        clock: fixed_now,
    }
}

pub(super) fn scope_with<Adjust>(table: DailyActivityTable, adjust: Adjust) -> DailyActivityScope
where
    Adjust: FnOnce(&mut DailyActivityScopeInput),
{
    let base = scope(table);
    let mut input = AsRef::<DailyActivityScopeInput>::as_ref(&base).clone();
    adjust(&mut input);
    DailyActivityScope::try_from(input).unwrap()
}

pub(super) struct Usage<'row> {
    pub(super) id: &'row str,
    pub(super) entity: Option<&'row str>,
    pub(super) date: &'row str,
    pub(super) api_key: &'row str,
    pub(super) model: Option<&'row str>,
    pub(super) spend: Decimal,
    pub(super) prompt: i64,
    pub(super) completion: i64,
    pub(super) cache_read: i64,
}

pub(super) fn usage<'row>(
    id: &'row str,
    entity: Option<&'row str>,
    date: &'row str,
    api_key: &'row str,
    spend: f64,
) -> Usage<'row> {
    Usage {
        id,
        entity,
        date,
        api_key,
        model: None,
        spend: Decimal::from_f64(spend).unwrap(),
        prompt: 10,
        completion: 5,
        cache_read: 0,
    }
}

pub(super) async fn insert(client: &Client, table: DailyActivityTable, row: &Usage<'_>) {
    let metadata = table.metadata();
    let sql = format!(
        "INSERT INTO \"{}\" (id, \"{}\", date, api_key, model, spend, prompt_tokens, completion_tokens, cache_read_input_tokens, api_requests, successful_requests, failed_requests, updated_at) VALUES ($1, $2, $3, $4, $5, $6::text::float8, $7::bigint, $8::bigint, $9::bigint, 1, 1, 0, NOW())",
        metadata.relation, metadata.entity_field
    );
    client
        .execute(
            sql.as_str(),
            &[
                &row.id,
                &row.entity,
                &row.date,
                &row.api_key,
                &row.model,
                &row.spend.to_string(),
                &row.prompt,
                &row.completion,
                &row.cache_read,
            ],
        )
        .await
        .unwrap();
}

pub(super) async fn insert_all<'row, Rows>(client: &Client, table: DailyActivityTable, rows: Rows)
where
    Rows: IntoIterator<Item = Usage<'row>>,
{
    for row in rows {
        insert(client, table, &row).await;
    }
}

pub(super) async fn texts(client: &Client, sql: &str) -> Vec<String> {
    let rows = client.query(sql, &[]).await.unwrap();
    rows.iter().map(|row| row.get::<_, String>(0)).collect()
}
