#![cfg(test)]

use super::{
    IdentityRowsDecodeError, decode_identity_rows, end_user::LiteLLMEndUserTable,
    key::LiteLLMVerificationToken, user::LiteLLMUserTable,
};
use crate::budget_permission_rows::JsonData;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use std::fs;
use std::path::PathBuf;
use tokio::task::JoinSet;

type ConnectionTasks = JoinSet<Result<(), tokio_postgres::Error>>;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    let environment = Environment::parse_from(["postgres-tests"]);
    environment.url.map(SecretString::from)
}

#[test]
fn identity_rows_defaults_and_password_exclusion() {
    let user: LiteLLMUserTable = serde_json::from_str(
        r#"{"user_id":"u","password":"secret","spend":null,"models":null,"teams":null}"#,
    )
    .unwrap();
    assert_eq!(user.spend, Decimal::ZERO);
    assert!(serde_json::to_string(&user).unwrap().contains("user_id"));
    assert!(!serde_json::to_string(&user).unwrap().contains("secret"));
}

#[test]
fn identity_money_nonfinite_and_overflow_rejected() {
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"spend":null}"#).unwrap_err();
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"spend":1e1000}"#).unwrap_err();
}

#[test]
fn native_identity_row_size_and_depth_limits() {
    let oversized = vec![b' '; 0x0010_0001];
    assert!(matches!(
        decode_identity_rows(&oversized),
        Err(IdentityRowsDecodeError::TooLarge { .. })
    ));
    let nested = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(
        decode_identity_rows(nested.as_bytes()),
        Err(IdentityRowsDecodeError::TooDeep)
    ));
}

#[rstest]
fn replays_the_identity_rows_corpus(#[files("fuzz/corpus/identity_rows/*")] path: PathBuf) {
    drop(decode_identity_rows(&fs::read(path).unwrap()));
}

#[tokio::test]
async fn postgres_identity_rows_preserve_storage_types() {
    let (fixture, client, mut tasks) = setup_identity_database().await;
    let key_json = row_json(&client, "LiteLLM_VerificationToken").await;
    let user_json = row_json(&client, "LiteLLM_UserTable").await;
    let end_user_json = row_json(&client, "LiteLLM_EndUserTable").await;
    let key: LiteLLMVerificationToken = serde_json::from_str(&key_json).unwrap();
    let user: LiteLLMUserTable = serde_json::from_str(&user_json).unwrap();
    let end_user: LiteLLMEndUserTable = serde_json::from_str(&end_user_json).unwrap();
    assert_eq!(key.spend, Decimal::ZERO);
    assert_eq!(
        key.model_spend.get("m"),
        Some(&JsonData::Number(serde_json::Number::from(2_i32)))
    );
    assert_eq!(user.spend, Decimal::ZERO);
    assert_eq!(end_user.spend, Decimal::ZERO);
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
    fixture.close_async().await.unwrap();
}

async fn setup_identity_database() -> (PostgresFixture, tokio_postgres::Client, ConnectionTasks) {
    let url = postgres_test_url();
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute(include_str!(
            "../../../../tools/postgres-fixture/fixtures/baseline.sql"
        ))
        .await
        .unwrap();
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend, models, aliases, model_spend)
               VALUES ('k', 0.0, ARRAY['m'], '{"a": 1}', '{"m": 2}');
               INSERT INTO "LiteLLM_UserTable" (user_id, spend, metadata)
               VALUES ('u', 0.0, '{"x": true}');
               INSERT INTO "LiteLLM_EndUserTable" (user_id, blocked, spend)
               VALUES ('e', false, 0.0);"#,
        )
        .await
        .unwrap();
    (fixture, client, tasks)
}

async fn row_json(client: &tokio_postgres::Client, table: &str) -> String {
    let query = format!("SELECT row_to_json(row)::TEXT FROM \"{table}\" row");
    client.query_one(&query, &[]).await.unwrap().get(0)
}
