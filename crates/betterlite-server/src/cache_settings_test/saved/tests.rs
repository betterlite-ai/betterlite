#![cfg(test)]

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::{errors::DbError, session::PostgresStore};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use serde_json::json;
use tokio_postgres::NoTls;

use crate::cache_settings_test::support;
use crate::cache_settings_test::support::SavedReply;

use super::CACHE_CONFIG_ROW_ID;
use super::SavedCacheConfig;
use super::saved_settings;

const CACHE_CONFIG_SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_CacheConfig" (
    id TEXT PRIMARY KEY,
    cache_settings TEXT NOT NULL
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["cache-settings-test-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(
        url.as_ref(),
        vec![CACHE_CONFIG_SCHEMA.to_owned()],
    )
    .resolve()
    .await
    .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

#[tokio::test]
async fn saved_settings_decrypts_only_string_values() {
    let stored = support::parse(&format!(
        r#"{{"password":"{}","port":6379,"ssl":true,"redis_startup_nodes":[{{"host":"127.0.0.1","port":7000}}],"nested":{{"password":"plain"}}}}"#,
        support::sealed(support::SAVED_SECRET)
    ));
    let (_, repository) = support::fake_saved(SavedReply::Stored(stored));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert_eq!(settings["password"], support::SAVED_SECRET);
    assert_eq!(settings["port"], 6379_u16);
    assert_eq!(settings["ssl"], true);
    assert_eq!(settings["redis_startup_nodes"][0]["host"], "127.0.0.1");
    assert_eq!(settings["nested"]["password"], "plain");
}

#[tokio::test]
async fn saved_settings_keeps_a_value_it_cannot_decrypt() {
    let stored = support::parse(&format!(
        r#"{{"password":"not-a-ciphertext","host":"{}"}}"#,
        support::sealed("127.0.0.1")
    ));
    let (_, repository) = support::fake_saved(SavedReply::Stored(stored));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert_eq!(settings["password"], "not-a-ciphertext");
    assert_eq!(settings["host"], "127.0.0.1");
}

#[tokio::test]
async fn saved_settings_keeps_the_stored_text_without_a_signing_key() {
    let sealed = support::sealed(support::SAVED_SECRET);
    let (_, repository) = support::fake_saved(SavedReply::Stored(support::parse(&format!(
        r#"{{"password":"{sealed}"}}"#
    ))));
    let settings = saved_settings(repository.as_deref(), &support::cipher_with(None)).await;
    assert_eq!(settings["password"].as_str(), Some(sealed.as_str()));
}

#[tokio::test]
async fn saved_settings_is_empty_without_a_repository() {
    let settings = saved_settings(None, &support::cipher()).await;
    assert!(settings.is_empty());
}

#[tokio::test]
async fn saved_settings_is_empty_when_the_row_is_missing() {
    let (_, repository) = support::fake_saved(SavedReply::Missing);
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert!(settings.is_empty());
}

#[tokio::test]
async fn saved_settings_is_empty_when_the_repository_fails() {
    let (saved_row, repository) = support::fake_saved(SavedReply::Failed);
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert!(settings.is_empty());
    assert_eq!(saved_row.calls(), 1);
}

#[tokio::test]
async fn saved_settings_is_empty_when_the_row_does_not_parse() {
    let (_, repository) = support::fake_saved(SavedReply::Stored(json!(r#"{"password":"#)));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert!(settings.is_empty());
}

#[tokio::test]
async fn saved_settings_parses_a_stored_json_string() {
    let sealed = support::sealed(support::SAVED_SECRET);
    let text = format!(r#"{{"password":"{sealed}","port":6379}}"#);
    let (_, repository) = support::fake_saved(SavedReply::Stored(json!(text)));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert_eq!(settings["password"], support::SAVED_SECRET);
    assert_eq!(settings["port"], 6379_u16);
}

#[rstest]
#[case::number(Value::from(5_u8))]
#[case::array(Value::Array(vec![Value::from(1_u8)]))]
#[case::flag(json!(true))]
#[case::quoted(json!(r#""text""#))]
#[tokio::test]
async fn saved_settings_is_empty_for_a_stored_non_object(#[case] stored: Value) {
    let (_, repository) = support::fake_saved(SavedReply::Stored(stored));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert!(settings.is_empty());
}

#[tokio::test]
async fn postgres_adapter_reports_a_missing_database() {
    let store = PostgresStore { pool: None };
    let error = SavedCacheConfig::cache_settings(&store)
        .await
        .expect_err("no pool, no row");
    assert!(matches!(
        error,
        TableError::Database(DbError::NoDbConnected)
    ));
}

#[tokio::test]
async fn postgres_adapter_reads_the_cache_config_row() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let text = r#"{"type":"redis","host":"127.0.0.1"}"#;
    client
        .execute(
            r#"INSERT INTO "LiteLLM_CacheConfig" (id, cache_settings) VALUES ($1, $2)"#,
            &[&CACHE_CONFIG_ROW_ID, &text],
        )
        .await
        .unwrap();
    drop(client);
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let stored = SavedCacheConfig::cache_settings(&store).await.unwrap();
    assert_eq!(stored, Some(support::parse(text)));
    pool.close();
    fixture.close_async().await.unwrap();
}
