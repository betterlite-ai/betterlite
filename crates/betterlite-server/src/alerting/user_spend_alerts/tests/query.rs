#![cfg(test)]

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::sql::READ_SPEND_TABLES;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::UserSpendSource as _;
use super::postgres::PgUserSpendSource;
use super::support::date;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["user-spend-alerts-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(
        url.as_ref(),
        vec![READ_SPEND_TABLES.to_owned()],
    )
    .resolve()
    .await
    .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    (fixture, pool, client)
}

fn source(pool: &Pool) -> PgUserSpendSource {
    PgUserSpendSource {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

#[tokio::test]
async fn query_sums_daily_monthly_and_baseline_per_user() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('user-1', '2025-12-31', 100.0, 1),
            ('user-1', '2026-01-01', 3.0, 1),
            ('user-1', '2026-01-08', 4.0, 1),
            ('user-1', '2026-01-15', 5.0, 1);
    "#,
        )
        .await
        .unwrap();
    drop(client);
    let rows = source(&pool).rows(date("2026-01-15"), 7).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user_id, "user-1");
    assert_eq!(rows[0].daily_spend, 5.0);
    assert_eq!(rows[0].monthly_spend, 12.0);
    assert_eq!(rows[0].baseline_spend, 4.0);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn query_leaves_out_null_users_and_users_without_monthly_spend() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            (NULL, '2026-01-10', 7.0, 1),
            ('user-2', '2026-01-10', 0.0, 1),
            ('user-3', '2026-01-10', 2.0, 1),
            ('user-4', '2025-12-01', 9.0, 1);
    "#,
        )
        .await
        .unwrap();
    drop(client);
    let rows = source(&pool).rows(date("2026-01-15"), 7).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user_id, "user-3");
    assert_eq!(rows[0].monthly_spend, 2.0);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn query_uses_the_baseline_window_before_today() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('user-1', '2026-01-13', 9.0, 1),
            ('user-1', '2026-01-14', 2.0, 1),
            ('user-1', '2026-01-15', 1.0, 1);
    "#,
        )
        .await
        .unwrap();
    drop(client);
    let rows = source(&pool).rows(date("2026-01-15"), 1).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].baseline_spend, 2.0);
    assert_eq!(rows[0].daily_spend, 1.0);
    assert_eq!(rows[0].monthly_spend, 12.0);
    let minimum = source(&pool).rows(date("2026-01-15"), 0).await.unwrap();
    assert_eq!(minimum[0].baseline_spend, 2.0);
    pool.close();
    fixture.close_async().await.unwrap();
}
