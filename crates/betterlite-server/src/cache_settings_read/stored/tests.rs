#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secrets::secret_codec::StoredValue;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use tokio_postgres::NoTls;

use super::{DatabaseCacheConfigStore, StoredSettingsError, stored_from_raw, stored_settings};
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::cache_settings_read::route::{CacheSettingsReadState, router, stored_cipher};
use crate::cache_settings_read::test_support::{
    FakeCacheConfigStore, StoreReply, cipher_settings, get, lookup, obj, parse,
};
use crate::cache_settings_read::view::effective_values;

fn sealed(cipher: &DbCipher, text: &str) -> String {
    let sealed = cipher
        .encrypt_value(
            StoredValue::<Value>::Text(SecretString::from(text.to_owned())),
            None,
        )
        .expect("the cipher seals the value");
    let StoredValue::Text(value) = sealed else {
        panic!("a sealed text stays text");
    };
    value.expose_secret().to_owned()
}

fn raw_settings(text: &str) -> Value {
    Value::Object(obj(text))
}

#[tokio::test]
async fn a_missing_row_gives_no_stored_settings() {
    let store = FakeCacheConfigStore::new(StoreReply::Missing);
    let cipher = stored_cipher(cipher_settings(None));
    let stored = stored_settings(&store, &cipher).await.unwrap();
    assert!(stored.is_empty());
}

#[rstest]
#[case::null(parse("null"))]
#[case::empty_text(parse(r#""""#))]
#[case::empty_object(parse("{}"))]
#[case::empty_list(parse("[]"))]
#[case::zero(parse("0"))]
#[case::false_flag(parse("false"))]
fn a_falsy_stored_column_gives_no_settings(#[case] raw: Value) {
    let cipher = stored_cipher(cipher_settings(None));
    let stored = stored_from_raw(&raw, &cipher).unwrap();
    assert!(stored.is_empty());
}

#[test]
fn a_stored_json_string_is_parsed_like_a_mapping() {
    let cipher = stored_cipher(cipher_settings(None));
    let raw = Value::String(r#"{"host": "string-host", "port": 6379}"#.to_owned());
    assert_eq!(
        stored_from_raw(&raw, &cipher).unwrap(),
        obj(r#"{"host": "string-host", "port": 6379}"#)
    );
}

#[test]
fn a_stored_mapping_is_used_as_it_arrives() {
    let cipher = stored_cipher(cipher_settings(None));
    let raw =
        raw_settings(r#"{"host": "object-host", "redis_startup_nodes": [["127.0.0.1", 7000]]}"#);
    assert_eq!(
        stored_from_raw(&raw, &cipher).unwrap(),
        obj(r#"{"host": "object-host", "redis_startup_nodes": [["127.0.0.1", 7000]]}"#)
    );
}

#[test]
fn a_stored_json_string_that_is_not_a_mapping_gives_no_settings() {
    let cipher = stored_cipher(cipher_settings(None));
    let raw = Value::String(r#"["host"]"#.to_owned());
    assert!(stored_from_raw(&raw, &cipher).unwrap().is_empty());
}

#[test]
fn an_encrypted_value_decrypts_to_its_plaintext() {
    let cipher = stored_cipher(cipher_settings(Some("sk-test")));
    let password = sealed(&cipher, "hunter2");
    let raw = raw_settings(&format!(r#"{{"password": "{password}"}}"#));
    assert_eq!(
        stored_from_raw(&raw, &cipher).unwrap(),
        obj(r#"{"password": "hunter2"}"#)
    );
}

#[test]
fn a_value_that_does_not_decrypt_stays_as_stored() {
    let cipher = stored_cipher(cipher_settings(Some("sk-test")));
    let foreign = sealed(&stored_cipher(cipher_settings(Some("sk-other"))), "hunter2");
    let raw = raw_settings(&format!(
        r#"{{"url": "{foreign}", "password": "plaintext"}}"#
    ));
    assert_eq!(
        stored_from_raw(&raw, &cipher).unwrap(),
        obj(&format!(
            r#"{{"url": "{foreign}", "password": "plaintext"}}"#
        ))
    );
}

#[test]
fn a_non_string_value_passes_through_undecrypted() {
    let cipher = stored_cipher(cipher_settings(Some("sk-test")));
    let raw = raw_settings(r#"{"port": 6379, "redis_startup_nodes": [["h", 7000]], "ssl": true}"#);
    assert_eq!(
        stored_from_raw(&raw, &cipher).unwrap(),
        obj(r#"{"port": 6379, "redis_startup_nodes": [["h", 7000]], "ssl": true}"#)
    );
}

#[test]
fn a_stored_encrypted_value_decrypts_before_the_environment_overlay() {
    let cipher = stored_cipher(cipher_settings(Some("sk-test")));
    let host = sealed(&cipher, "stored-host");
    let raw = raw_settings(&format!(r#"{{"host": "{host}"}}"#));
    let stored = stored_from_raw(&raw, &cipher).unwrap();
    let current = effective_values(
        &stored,
        lookup(&[("REDIS_HOST", "environment-host")]).as_ref(),
    );
    assert_eq!(
        current.get("host"),
        Some(&Value::String("stored-host".to_owned()))
    );
}

#[test]
fn the_environment_is_never_decrypted() {
    let cipher = stored_cipher(cipher_settings(Some("sk-test")));
    let host = sealed(&cipher, "environment-host");
    let stored = stored_from_raw(&raw_settings("{}"), &cipher).unwrap();
    let current = effective_values(&stored, lookup(&[("REDIS_HOST", &host)]).as_ref());
    assert_eq!(current.get("host"), Some(&Value::String(host)));
}

#[test]
fn invalid_stored_json_is_an_error() {
    let cipher = stored_cipher(cipher_settings(None));
    let raw = Value::String("{not json".to_owned());
    let error = stored_from_raw(&raw, &cipher).expect_err("invalid JSON is an error");
    assert!(matches!(error, StoredSettingsError::Parse(_)));
}

#[tokio::test]
async fn a_repository_error_is_an_error() {
    let store = FakeCacheConfigStore::new(StoreReply::Failing);
    let cipher = stored_cipher(cipher_settings(None));
    let error = stored_settings(&store, &cipher)
        .await
        .expect_err("a failed read is an error");
    assert!(matches!(error, StoredSettingsError::Table(_)));
}

#[rstest]
#[case::repository_error(StoreReply::Failing)]
#[case::invalid_stored_json(StoreReply::Settings(Value::String("{not json".to_owned())))]
#[tokio::test]
async fn a_stored_settings_failure_answers_500_with_the_error_prefix(#[case] reply: StoreReply) {
    let state = CacheSettingsReadState::new(
        Some(Arc::new(FakeCacheConfigStore::new(reply))),
        cipher_settings(None),
        lookup(&[]),
    );
    let (status, body) = get(router(state), Some(master_caller())).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let parsed = parse(&body);
    let detail = parsed
        .get("detail")
        .and_then(Value::as_str)
        .expect("the detail member");
    assert!(
        detail.starts_with("Error fetching cache settings: "),
        "{detail}"
    );
    assert_eq!(parsed.as_object().expect("an object body").len(), 1);
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_CacheConfig" (
    id TEXT PRIMARY KEY DEFAULT 'cache_config',
    cache_settings JSONB NOT NULL,
    created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

const INSERT_SQL: &str =
    r#"INSERT INTO "LiteLLM_CacheConfig" (id, cache_settings) VALUES ($1, $2::text::jsonb)"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["cache-settings-read-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

fn store(pool: &Pool) -> Arc<PostgresStore> {
    Arc::new(PostgresStore {
        pool: Some(pool.clone()),
    })
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn insert_row(pool: &Pool, id: &str, cache_settings: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(INSERT_SQL, &[&id, &cache_settings])
        .await
        .unwrap();
}

#[tokio::test]
async fn the_cache_config_row_is_read_from_the_database() {
    let (fixture, pool) = database().await;
    insert_row(
        &pool,
        "cache_config",
        r#"{"host": "postgres-host", "port": 6379}"#,
    )
    .await;
    insert_row(&pool, "other", r#"{"host": "other-host"}"#).await;
    let cipher = stored_cipher(cipher_settings(None));
    let found = stored_settings(
        &DatabaseCacheConfigStore {
            store: store(&pool),
            availability: Availability::default(),
        },
        &cipher,
    )
    .await
    .unwrap();
    assert_eq!(found, obj(r#"{"host": "postgres-host", "port": 6379}"#));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_database_without_the_row_gives_no_stored_settings() {
    let (fixture, pool) = database().await;
    insert_row(&pool, "other", r#"{"host": "other-host"}"#).await;
    let cipher = stored_cipher(cipher_settings(None));
    let found = stored_settings(
        &DatabaseCacheConfigStore {
            store: store(&pool),
            availability: Availability::default(),
        },
        &cipher,
    )
    .await
    .unwrap();
    assert!(found.is_empty());
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_database_row_holding_a_json_string_is_parsed() {
    let (fixture, pool) = database().await;
    insert_row(&pool, "cache_config", r#""{\"host\": \"string-host\"}""#).await;
    let cipher = stored_cipher(cipher_settings(None));
    let found = stored_settings(
        &DatabaseCacheConfigStore {
            store: store(&pool),
            availability: Availability::default(),
        },
        &cipher,
    )
    .await
    .unwrap();
    assert_eq!(found, obj(r#"{"host": "string-host"}"#));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_database_row_with_an_encrypted_value_decrypts_it() {
    let (fixture, pool) = database().await;
    let cipher = stored_cipher(cipher_settings(Some("sk-test")));
    let password = sealed(&cipher, "hunter2");
    insert_row(
        &pool,
        "cache_config",
        &format!(r#"{{"password": "{password}"}}"#),
    )
    .await;
    let found = stored_settings(
        &DatabaseCacheConfigStore {
            store: store(&pool),
            availability: Availability::default(),
        },
        &cipher,
    )
    .await
    .unwrap();
    assert_eq!(found, obj(r#"{"password": "hunter2"}"#));
    close(fixture, pool).await;
}
