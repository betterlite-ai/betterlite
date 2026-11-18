#![cfg(test)]

use std::sync::Arc;

use betterlite_db::database_faults::ErrorType;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::{CacheConfigRepository as _, DatabaseCacheConfigStore};
use crate::cache_settings_update::support;

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
    let url = Environment::parse_from(["cache-settings-update-tests"])
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

fn repository(pool: &Pool) -> DatabaseCacheConfigStore {
    DatabaseCacheConfigStore {
        store: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
        availability: Availability::default(),
    }
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn the_repository_reads_the_cache_config_row() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    client
        .execute(INSERT_SQL, &[&"cache_config", &r#"{"host": "db-host"}"#])
        .await
        .unwrap();
    drop(client);
    let stored = repository(&pool).stored_cache_settings().await.unwrap();
    assert_eq!(stored, Some(support::json(r#"{"host": "db-host"}"#)));
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_repository_answers_none_without_the_row() {
    let (fixture, pool) = database().await;
    let stored = repository(&pool).stored_cache_settings().await.unwrap();
    assert!(stored.is_none());
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_repository_reports_a_closed_pool_as_a_connect_fault() {
    let (fixture, pool) = database().await;
    let repository = repository(&pool);
    pool.close();
    let error = repository.stored_cache_settings().await.unwrap_err();
    assert_eq!(error.error_type, ErrorType::Connect);
    assert_eq!(error.message, "database pool unavailable");
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_repository_creates_and_updates_the_row() {
    let (fixture, pool) = database().await;
    let repository = repository(&pool);
    repository
        .upsert_cache_settings(r#"{"host": "first-host"}"#)
        .await
        .unwrap();
    let created = repository.stored_cache_settings().await.unwrap();
    assert_eq!(created, Some(support::json(r#"{"host": "first-host"}"#)));
    repository
        .upsert_cache_settings(r#"{"host": "second-host"}"#)
        .await
        .unwrap();
    let updated = repository.stored_cache_settings().await.unwrap();
    assert_eq!(updated, Some(support::json(r#"{"host": "second-host"}"#)));
    close(fixture, pool).await;
}
