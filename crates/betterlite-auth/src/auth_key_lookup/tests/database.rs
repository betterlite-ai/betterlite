#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_db::combined_key_read::error::{DatabaseFailure, KeyReadError};
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_faults::ErrorType;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::SecretString;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use super::support::{FakeSource, reader};
use crate::auth_key_lookup::error::KeyLookupError;
use crate::auth_key_lookup::jwt::{
    DatabaseJwtMappings, JwtError, JwtMappingSource as _, get_jwt_key_mapping_object,
};

const SCHEMA: &str = r#"CREATE TABLE "LiteLLM_JWTKeyMapping" (
    id text, token text, jwt_issuer text, jwt_claim_name text, jwt_claim_value text, is_active boolean);
INSERT INTO "LiteLLM_JWTKeyMapping" VALUES
    ('a', 'scoped', 'issuer', 'group', 'dev', true),
    ('b', 'global', '', 'group', 'dev', true),
    ('c', 'inactive', 'other', 'group', 'dev', false);"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let configured = Environment::parse_from(["key-lookup-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(configured.as_ref(), vec![SCHEMA.to_owned()])
            .resolve()
            .await
            .unwrap();
    let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
        .max_size(2)
        .build()
        .unwrap();
    (fixture, pool)
}

fn mapping_source(pool: Option<Pool>) -> DatabaseJwtMappings {
    DatabaseJwtMappings {
        reader: PostgresStore { pool },
        default_deadline_seconds: 5.0,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }
}

#[tokio::test]
async fn mapping_repository_scopes() {
    let (fixture, pool) = database().await;
    let source = mapping_source(Some(pool.clone()));
    assert_eq!(
        get_jwt_key_mapping_object("group", "dev", Some("issuer"), &source)
            .await
            .unwrap()
            .as_deref(),
        Some("scoped")
    );
    assert_eq!(
        get_jwt_key_mapping_object("group", "dev", None, &source)
            .await
            .unwrap()
            .as_deref(),
        Some("global")
    );
    assert!(
        get_jwt_key_mapping_object("group", "dev", Some("other"), &source)
            .await
            .unwrap()
            .is_none()
    );
    let rows = source
        .mappings_for_tokens(&["scoped".to_owned()])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].token, "scoped");
    assert_eq!(rows[0].jwt_issuer, "issuer");
    assert_eq!(rows[0].jwt_claim_name, "group");
    assert_eq!(rows[0].jwt_claim_value, "dev");
    source.tasks.close();
    source.tasks.wait().await;
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn mapping_database_failure() {
    let source = mapping_source(None);
    assert!(source.mappings_for_tokens(&[]).await.unwrap().is_empty());
    let error = source
        .mappings_for_tokens(&["token".to_owned()])
        .await
        .unwrap_err();
    let JwtError::Lookup(DbLookupError::Database(fault)) = error else {
        panic!("database failure expected");
    };
    assert_eq!(fault.error_type, ErrorType::Prisma);
    assert!(!fault.message.is_empty());
    get_jwt_key_mapping_object("group", "dev", None, &source)
        .await
        .unwrap_err();
}

#[rstest]
#[case::data("SELECT 1 / 0", false)]
#[case::transport(
    "DO $$ BEGIN RAISE EXCEPTION 'lost' USING ERRCODE = '08006'; END $$",
    true
)]
#[tokio::test]
async fn driver_sqlstate(#[case] sql: &str, #[case] transport: bool) {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let driver = client.batch_execute(sql).await.unwrap_err();
    let source = FakeSource::new(vec![Err(KeyReadError::Database(DatabaseFailure::Driver(
        Arc::new(DriverError { driver }),
    )))]);
    let error = reader(source, None)
        .fetch("hash", false, None)
        .await
        .unwrap_err();
    assert_eq!(
        matches!(error, KeyLookupError::Lookup(DbLookupError::Database(_))),
        transport
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn driver_closed_connection() {
    let (fixture, pool) = database().await;
    let (client, connection) = fixture.options.config.connect(NoTls).await.unwrap();
    let tasks = TaskTracker::new();
    let handle = tasks.spawn(connection);
    handle.abort();
    assert!(handle.await.unwrap_err().is_cancelled());
    let driver = client.simple_query("SELECT 1").await.unwrap_err();
    assert!(driver.is_closed());
    let source = FakeSource::new(vec![Err(KeyReadError::Database(DatabaseFailure::Driver(
        Arc::new(DriverError { driver }),
    )))]);
    let error = reader(source, None)
        .fetch("hash", false, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        KeyLookupError::Lookup(DbLookupError::Database(_))
    ));
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}
