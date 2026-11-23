use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_db::database_replica::client::{Attribute, DatabaseClient as _};
use betterlite_db::key_hash::KeyHash;
use betterlite_db::key_repository::table::TOKEN_TABLE_ATTRIBUTE;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;

use super::support::strings;
use crate::key_list_filters::error::{KeyListFiltersError, KeyListReadError};
use crate::key_list_filters::ports::KeyListFiltersStore as _;
use crate::key_list_filters::postgres::{PostgresKeyListFiltersStore, PostgresTokenClient};

const SCHEMA_SQL: &str = r#"
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    teams JSONB
);
CREATE TABLE "LiteLLM_OrganizationMembership" (
    user_id TEXT NOT NULL,
    organization_id TEXT NOT NULL,
    user_role TEXT,
    created_at TIMESTAMP(3) NOT NULL,
    updated_at TIMESTAMP(3) NOT NULL
);
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    user_id TEXT,
    key_alias TEXT
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["key-list-filters-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA_SQL.to_owned()])
            .resolve()
            .await
            .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

fn filters_store(pool: &Pool) -> PostgresKeyListFiltersStore {
    PostgresKeyListFiltersStore {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

#[tokio::test]
async fn postgres_reads_user_row() {
    let (_fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_UserTable" (user_id, teams) VALUES
            ('u1', '["t1","t2"]'::jsonb),
            ('u2', NULL);
        "#,
    )
    .await;
    let store = filters_store(&pool);
    let row = store.user_row("u1").await.unwrap().unwrap();
    assert_eq!(row.user_id, "u1");
    assert_eq!(row.teams, strings(&["t1", "t2"]));
    let bare = store.user_row("u2").await.unwrap().unwrap();
    assert!(bare.teams.is_empty());
    // the read matches the stored id exactly, and a missing row is None rather than an error
    assert!(store.user_row("U1").await.unwrap().is_none());
    assert!(store.user_row("u3").await.unwrap().is_none());
}

#[tokio::test]
async fn postgres_reads_key_row() {
    let (_fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_VerificationToken" (token, user_id, key_alias) VALUES
            ('sk-hash-1', 'u1', 'first'),
            ('sk-hash-2', 'u2', NULL);
        "#,
    )
    .await;
    let store = filters_store(&pool);
    let found = store
        .key_by_hash(&KeyHash::from("sk-hash-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.user_id.as_deref(), Some("u1"));
    assert_eq!(found.key_alias.as_deref(), Some("first"));
    let second = store
        .key_by_hash(&KeyHash::from("sk-hash-2"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.user_id.as_deref(), Some("u2"));
    assert!(second.key_alias.is_none());
    // an exact token match only: a different case and an absent hash find nothing
    assert!(
        store
            .key_by_hash(&KeyHash::from("SK-HASH-1"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .key_by_hash(&KeyHash::from("sk-hash-9"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn postgres_without_pool_reports_read_failures() {
    let store = PostgresKeyListFiltersStore {
        store: PostgresStore { pool: None },
    };
    let user = store.user_row("u1").await;
    assert!(matches!(
        user,
        Err(KeyListFiltersError::Read(KeyListReadError::User(_)))
    ));
    let key = store.key_by_hash(&KeyHash::from("sk-hash-1")).await;
    assert!(matches!(
        key,
        Err(KeyListFiltersError::Read(KeyListReadError::Key(_)))
    ));
}

#[tokio::test]
async fn postgres_token_client_serves_only_its_table() {
    let (_fixture, pool) = database().await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let client = PostgresTokenClient { store: &store };
    assert!(matches!(
        client.attribute(TOKEN_TABLE_ATTRIBUTE).unwrap(),
        Some(Attribute::Model(_))
    ));
    assert!(client.attribute("litellm_usertable").unwrap().is_none());
    let disconnected = PostgresStore { pool: None };
    let client = PostgresTokenClient {
        store: &disconnected,
    };
    assert!(client.attribute(TOKEN_TABLE_ATTRIBUTE).unwrap().is_none());
}
