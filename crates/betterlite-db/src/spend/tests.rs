#![cfg(test)]

use super::read_spend;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDate;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["spend-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client
        .batch_execute(include_str!("schema.sql"))
        .await
        .unwrap();
    (fixture, pool, client)
}

fn date(value: &str) -> NaiveDate {
    value.parse().unwrap()
}

#[tokio::test]
async fn aggregates_spend_and_requests_per_user_and_date() {
    let (fixture, pool, client) = database().await;
    client.batch_execute(r#"
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('user-1', 'User.One@Example.com');
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('user-1', '2026-01-01', 1.5, 2),
            ('user-1', '2026-01-01', 2.5, 3);
    "#).await.unwrap();
    let records = read_spend(&**client, date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].date, "2026-01-01");
    assert_eq!(records[0].user_id, "user-1");
    assert_eq!(records[0].email, "user.one@example.com");
    assert_eq!(records[0].spend, Decimal::from(4_i32));
    assert_eq!(records[0].requests, 5);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn results_are_ordered_by_date_ascending() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('user-1', '2026-01-03', 1.0, 1),
            ('user-1', '2026-01-01', 1.0, 1),
            ('user-1', '2026-01-02', 1.0, 1);
    "#,
        )
        .await
        .unwrap();
    let records = read_spend(&**client, date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    let dates: Vec<&str> = records.iter().map(|record| record.date.as_str()).collect();
    assert_eq!(dates, vec!["2026-01-01", "2026-01-02", "2026-01-03"]);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn date_range_is_inclusive_on_both_ends() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('user-1', '2025-12-31', 1.0, 1),
            ('user-1', '2026-01-01', 1.0, 1),
            ('user-1', '2026-01-31', 1.0, 1),
            ('user-1', '2026-02-01', 1.0, 1);
    "#,
        )
        .await
        .unwrap();
    let records = read_spend(&**client, date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    let dates: Vec<&str> = records.iter().map(|record| record.date.as_str()).collect();
    assert_eq!(dates, vec!["2026-01-01", "2026-01-31"]);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn missing_user_record_falls_back_to_user_id() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('ghost@example.com', '2026-01-01', 1.0, 1),
            ('plain-user', '2026-01-01', 1.0, 1);
    "#,
        )
        .await
        .unwrap();
    let records = read_spend(&**client, date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(records.len(), 2);
    let ghost = records
        .iter()
        .find(|record| record.user_id == "ghost@example.com")
        .unwrap();
    assert_eq!(ghost.email, "ghost@example.com");
    let plain = records
        .iter()
        .find(|record| record.user_id == "plain-user")
        .unwrap();
    assert_eq!(plain.email, "");
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn null_user_email_falls_back_to_user_id() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('user-1', NULL);
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('user-1', '2026-01-01', 1.0, 1);
    "#,
        )
        .await
        .unwrap();
    let records = read_spend(&**client, date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].email, "");
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn null_user_id_group_falls_back_to_empty_string() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            (NULL, '2026-01-01', 1.0, 1);
    "#,
        )
        .await
        .unwrap();
    let records = read_spend(&**client, date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].user_id, "");
    assert_eq!(records[0].email, "");
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn no_matching_rows_returns_empty() {
    let (fixture, pool, client) = database().await;
    let records = read_spend(&**client, date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert!(records.is_empty());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}
