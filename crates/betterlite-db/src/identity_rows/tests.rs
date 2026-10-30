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
    assert_eq!(user.password.as_ref().unwrap().expose_secret(), "secret");
    assert!(serde_json::to_string(&user).unwrap().contains("user_id"));
    assert!(!serde_json::to_string(&user).unwrap().contains("secret"));
    assert!(!format!("{user:?}").contains("secret"));
}

#[test]
fn identity_money_serializes_as_float_like_pydantic() {
    let key: LiteLLMVerificationToken =
        serde_json::from_str(r#"{"spend":1.25,"total_spend":10,"max_budget":5}"#).unwrap();
    let json = serde_json::to_string(&key).unwrap();
    assert!(json.contains(r#""spend":1.25"#));
    assert!(json.contains(r#""total_spend":10.0"#));
    assert!(json.contains(r#""max_budget":5.0"#));
    let defaulted: LiteLLMVerificationToken = serde_json::from_str("{}").unwrap();
    assert!(
        serde_json::to_string(&defaulted)
            .unwrap()
            .contains(r#""spend":0.0"#)
    );
}

#[test]
fn identity_money_nonfinite_and_overflow_rejected() {
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"spend":null}"#).unwrap_err();
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"spend":1e1000}"#).unwrap_err();
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"spend":1e30}"#).unwrap_err();
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"total_spend":1e30}"#).unwrap_err();
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"max_budget":1e30}"#).unwrap_err();
    let oversized = "9".repeat(60);
    serde_json::from_str::<LiteLLMVerificationToken>(&format!(r#"{{"spend":{oversized}}}"#))
        .unwrap_err();
    serde_json::from_str::<LiteLLMUserTable>(&format!(r#"{{"user_id":"u","spend":{oversized}}}"#))
        .unwrap_err();
    serde_json::from_str::<LiteLLMEndUserTable>(&format!(
        r#"{{"user_id":"u","blocked":false,"spend":{oversized}}}"#
    ))
    .unwrap_err();
}

#[test]
fn identity_rows_reject_duplicate_payload_keys() {
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"aliases":{"a":1,"a":2}}"#).unwrap_err();
    serde_json::from_str::<LiteLLMUserTable>(r#"{"user_id":"u","metadata":{"a":1,"a":2}}"#)
        .unwrap_err();
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
    let key_json = row_json(&client, "LiteLLM_VerificationToken", "k").await;
    let key_zero_json = row_json(&client, "LiteLLM_VerificationToken", "k-zero").await;
    let key_null_models_json =
        row_json(&client, "LiteLLM_VerificationToken", "k-null-models").await;
    let user_json = row_json(&client, "LiteLLM_UserTable", "u").await;
    let user_zero_json = row_json(&client, "LiteLLM_UserTable", "u-zero").await;
    let end_user_json = row_json(&client, "LiteLLM_EndUserTable", "e").await;
    let end_user_zero_json = row_json(&client, "LiteLLM_EndUserTable", "e-zero").await;

    let key: LiteLLMVerificationToken = serde_json::from_str(&key_json).unwrap();
    let key_zero: LiteLLMVerificationToken = serde_json::from_str(&key_zero_json).unwrap();
    let user: LiteLLMUserTable = serde_json::from_str(&user_json).unwrap();
    let user_zero: LiteLLMUserTable = serde_json::from_str(&user_zero_json).unwrap();
    let end_user: LiteLLMEndUserTable = serde_json::from_str(&end_user_json).unwrap();
    let end_user_zero: LiteLLMEndUserTable = serde_json::from_str(&end_user_zero_json).unwrap();
    serde_json::from_str::<LiteLLMVerificationToken>(&key_null_models_json).unwrap_err();

    assert_eq!(key.spend, Decimal::new(125, 2));
    assert_eq!(key.max_budget, Some(Decimal::new(5, 0)));
    assert_eq!(
        key.model_spend.0.get("m"),
        Some(&JsonData::Number(serde_json::Number::from(2_i32)))
    );
    assert_eq!(key_zero.spend, Decimal::ZERO);
    assert_eq!(key_zero.max_budget, None);
    assert_eq!(key_zero.aliases, super::Payload::default());
    assert!(key_zero.models.is_empty());
    assert_eq!(key_zero.model_spend, super::Payload::default());
    assert_eq!(user.spend, Decimal::new(250, 2));
    assert_eq!(user_zero.spend, Decimal::ZERO);
    assert_eq!(end_user.spend, Decimal::new(375, 2));
    assert_eq!(end_user_zero.spend, Decimal::ZERO);
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
            r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend, max_budget, models, aliases, model_spend)
               VALUES ('k', 1.25, 5.0, ARRAY['m'], '{"a": 1}', '{"m": 2}');
               INSERT INTO "LiteLLM_VerificationToken" (token, models)
               VALUES ('k-zero', ARRAY[]::TEXT[]);
               INSERT INTO "LiteLLM_VerificationToken" (token)
               VALUES ('k-null-models');
               INSERT INTO "LiteLLM_UserTable" (user_id, spend, metadata)
               VALUES ('u', 2.5, '{"x": true}');
               INSERT INTO "LiteLLM_UserTable" (user_id)
               VALUES ('u-zero');
               INSERT INTO "LiteLLM_EndUserTable" (user_id, blocked, spend)
               VALUES ('e', false, 3.75);
               INSERT INTO "LiteLLM_EndUserTable" (user_id, blocked)
               VALUES ('e-zero', false);"#,
        )
        .await
        .unwrap();
    (fixture, client, tasks)
}

async fn row_json(client: &tokio_postgres::Client, table: &str, key: &str) -> String {
    let column = if table == "LiteLLM_VerificationToken" {
        "token"
    } else {
        "user_id"
    };
    let query =
        format!("SELECT row_to_json(row)::TEXT FROM \"{table}\" row WHERE row.{column} = $1");
    client.query_one(&query, &[&key]).await.unwrap().get(0)
}
