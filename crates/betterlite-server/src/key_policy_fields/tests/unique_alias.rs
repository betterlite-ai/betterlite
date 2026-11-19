#![cfg(test)]

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::key_policy_fields::unique_alias::{AliasUniquenessError, enforce_unique_key_alias};

const KEY_TABLE_SQL: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    key_alias TEXT
);
INSERT INTO "LiteLLM_VerificationToken" (token, key_alias) VALUES
    ('sk-raw', 'taken'),
    ('sk-other', 'other');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["key-policy-fields-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![KEY_TABLE_SQL.to_owned()])
            .resolve()
            .await
            .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    (fixture, pool, client)
}

#[tokio::test]
async fn uniqueness_missing_db_noop() {
    enforce_unique_key_alias(Some("taken"), None, None)
        .await
        .unwrap();
    enforce_unique_key_alias(None, None, Some("sk-raw"))
        .await
        .unwrap();
}

#[tokio::test]
async fn uniqueness_global_excludes_raw_token() {
    let (fixture, pool, client) = database().await;
    drop(client);
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    // another key holds the alias, so the lookup collides
    let error = enforce_unique_key_alias(Some("taken"), Some(&store), None)
        .await
        .unwrap_err();
    let AliasUniquenessError::Duplicate(exception) = error else {
        panic!("a taken alias is a duplicate");
    };
    assert_eq!(
        exception.message,
        "Key with alias 'taken' already exists. Unique key aliases across all keys are required."
    );
    assert_eq!(exception.code, "400");
    assert_eq!(exception.error_type, "bad_request_error");
    assert_eq!(exception.param.as_deref(), Some("key_alias"));
    // the holding token exempts its own row, and an empty token exempts nothing
    enforce_unique_key_alias(Some("taken"), Some(&store), Some("sk-raw"))
        .await
        .unwrap();
    assert!(matches!(
        enforce_unique_key_alias(Some("taken"), Some(&store), Some("sk-other")).await,
        Err(AliasUniquenessError::Duplicate(_))
    ));
    assert!(matches!(
        enforce_unique_key_alias(Some("taken"), Some(&store), Some("")).await,
        Err(AliasUniquenessError::Duplicate(_))
    ));
    // a free alias and a missing alias both pass without a read
    enforce_unique_key_alias(Some("free"), Some(&store), None)
        .await
        .unwrap();
    enforce_unique_key_alias(None, Some(&store), None)
        .await
        .unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
}
