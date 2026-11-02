#![cfg(test)]

use super::{PostgresSpendReader, SpendReader as _};
use betterlite_db::postgres_store::session::PostgresStore;
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
    let url = Environment::parse_from(["roi-spend-tests"])
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
async fn reader_delegates_to_read_spend_and_normalizes_email() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('user-1', 'User.One@Example.com');
        INSERT INTO "LiteLLM_DailyUserSpend" (user_id, date, spend, api_requests) VALUES
            ('user-1', '2026-01-01', 1.5, 2);
    "#,
        )
        .await
        .unwrap();
    let reader = PostgresSpendReader {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    };
    let records = reader
        .read(date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].email, "user.one@example.com");
    assert_eq!(records[0].spend, "1.5".parse::<Decimal>().unwrap());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn reader_returns_empty_when_no_rows_match() {
    let (fixture, pool, client) = database().await;
    let reader = PostgresSpendReader {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    };
    let records = reader
        .read(date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert!(records.is_empty());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}
