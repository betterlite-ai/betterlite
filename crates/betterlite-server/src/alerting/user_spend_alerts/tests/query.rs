#![cfg(test)]

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::sql::READ_SPEND_TABLES;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::UserSpendSource as _;
use super::postgres::{PgUserSpendSource, UserSpendQueryError};
use super::support::{date, decimal};

// a user id column the query reads fine but the row's String field cannot decode
const INTEGER_USER_ID_TABLE: &str = r#"
    CREATE TABLE "LiteLLM_DailyUserSpend" (
        user_id integer,
        date text,
        spend double precision,
        api_requests bigint
    )
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database(statements: Vec<String>) -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["user-spend-alerts-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
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
    let (fixture, pool, client) = database(vec![READ_SPEND_TABLES.to_owned()]).await;
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
    assert_eq!(rows[0].daily_spend, decimal("5"));
    assert_eq!(rows[0].monthly_spend, decimal("12"));
    assert_eq!(rows[0].baseline_spend, decimal("4"));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn query_leaves_out_null_users_and_users_without_monthly_spend() {
    let (fixture, pool, client) = database(vec![READ_SPEND_TABLES.to_owned()]).await;
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
    assert_eq!(rows[0].monthly_spend, decimal("2"));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn query_uses_the_baseline_window_before_today() {
    let (fixture, pool, client) = database(vec![READ_SPEND_TABLES.to_owned()]).await;
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
    assert_eq!(rows[0].baseline_spend, decimal("2"));
    assert_eq!(rows[0].daily_spend, decimal("1"));
    assert_eq!(rows[0].monthly_spend, decimal("12"));
    let minimum = source(&pool).rows(date("2026-01-15"), 0).await.unwrap();
    assert_eq!(minimum[0].baseline_spend, decimal("2"));
    pool.close();
    fixture.close_async().await.unwrap();
}

// LiteLLM's TypeAdapter raises on a row that does not fit UserSpendRow; the source answers with an error
#[tokio::test]
async fn query_errors_on_a_row_that_does_not_decode() {
    let (fixture, pool, client) = database(vec![INTEGER_USER_ID_TABLE.to_owned()]).await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            (7, '2026-01-15', 5.0, 1);
    "#,
        )
        .await
        .unwrap();
    drop(client);
    let error = source(&pool).rows(date("2026-01-15"), 7).await.unwrap_err();
    assert!(matches!(error, UserSpendQueryError::Database(_)));
    pool.close();
    fixture.close_async().await.unwrap();
}

// LiteLLM's float row keeps a NaN or an infinity, and the HAVING keeps both; Decimal cannot hold either
#[rstest]
#[case::infinity("Infinity")]
#[case::nan("NaN")]
#[tokio::test]
async fn query_errors_on_a_non_finite_spend(#[case] spend: &str) {
    let (fixture, pool, client) = database(vec![READ_SPEND_TABLES.to_owned()]).await;
    client
        .batch_execute(&format!(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('user-1', '2026-01-15', '{spend}'::float8, 1);
    "#
        ))
        .await
        .unwrap();
    drop(client);
    let error = source(&pool).rows(date("2026-01-15"), 7).await.unwrap_err();
    assert!(matches!(
        error,
        UserSpendQueryError::Unrepresentable {
            name: "daily_spend"
        }
    ));
    pool.close();
    fixture.close_async().await.unwrap();
}
