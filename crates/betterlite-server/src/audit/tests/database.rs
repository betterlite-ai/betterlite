#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{NaiveDate, NaiveDateTime};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::support::record;
use crate::audit::repository::PostgresAuditLogRepository;
use crate::audit_events::sink::{AuditLogRepository as _, AuditRepositoryError};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS "LiteLLM_AuditLog" (
    "id" TEXT NOT NULL,
    "updated_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "changed_by" TEXT NOT NULL DEFAULT '',
    "changed_by_api_key" TEXT NOT NULL DEFAULT '',
    "action" TEXT NOT NULL,
    "table_name" TEXT NOT NULL,
    "object_id" TEXT NOT NULL,
    "before_value" JSONB,
    "updated_values" JSONB,
    CONSTRAINT "LiteLLM_AuditLog_pkey" PRIMARY KEY ("id")
)
"#;

const SELECT: &str = r#"
SELECT action, table_name, object_id, changed_by, changed_by_api_key,
       before_value::text, updated_values::text, updated_at
FROM "LiteLLM_AuditLog" WHERE id = $1
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["audit-delivery-tests"])
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

fn repository(pool: &Pool) -> PostgresAuditLogRepository {
    PostgresAuditLogRepository {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

fn row_timestamp() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2024, 5, 6)
        .and_then(|date| date.and_hms_milli_opt(7, 8, 9, 123))
        .expect("the timestamp exists")
}

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the text is JSON")
}

#[tokio::test]
async fn a_record_without_actor_or_key_keeps_the_column_defaults() {
    let (fixture, pool) = database().await;
    let repository = repository(&pool);
    let mut stored = record();
    stored.changed_by = None;
    stored.changed_by_api_key = None;
    stored.before_value = None;
    stored.updated_values = None;
    repository
        .insert_audit_log(&stored)
        .await
        .expect("the partial row inserts");
    let client = pool.get().await.unwrap();
    let row = client.query_one(SELECT, &[&"audit-1"]).await.unwrap();
    assert_eq!(row.get::<_, String>(0), "updated");
    assert_eq!(row.get::<_, String>(1), "LiteLLM_TeamTable");
    assert_eq!(row.get::<_, String>(2), "team-1");
    assert_eq!(row.get::<_, String>(3), "");
    assert_eq!(row.get::<_, String>(4), "");
    assert!(row.get::<_, Option<String>>(5).is_none());
    assert!(row.get::<_, Option<String>>(6).is_none());
    assert_eq!(row.get::<_, NaiveDateTime>(7), row_timestamp());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn a_full_record_round_trips_through_the_partial_insert() {
    let (fixture, pool) = database().await;
    let repository = repository(&pool);
    assert!(repository.available());
    let mut stored = record();
    stored.updated_values = Some(json(r#"{"api_key":"sk-1*****7890"}"#));
    repository
        .insert_audit_log(&stored)
        .await
        .expect("the full row inserts");
    let client = pool.get().await.unwrap();
    let row = client.query_one(SELECT, &[&"audit-1"]).await.unwrap();
    assert_eq!(row.get::<_, String>(3), "admin");
    assert_eq!(row.get::<_, String>(4), "sk-key");
    assert_eq!(
        json(row.get::<_, String>(5).as_str()),
        json(r#"{"api_key": "sk-1*****7890"}"#)
    );
    assert_eq!(
        json(row.get::<_, String>(6).as_str()),
        json(r#"{"api_key": "sk-1*****7890"}"#)
    );
    assert_eq!(row.get::<_, NaiveDateTime>(7), row_timestamp());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn an_absent_database_is_reported_as_a_table_error() {
    let repository = PostgresAuditLogRepository {
        store: PostgresStore { pool: None },
    };
    assert!(!repository.available());
    let error = repository
        .insert_audit_log(&record())
        .await
        .expect_err("there is no database");
    assert!(
        matches!(error, AuditRepositoryError::Table(TableError::Database(_))),
        "{error:?}"
    );
    assert!(error.to_string().contains("No DB Connected"));
}

#[tokio::test]
async fn a_duplicate_id_is_reported_as_a_database_error() {
    let (fixture, pool) = database().await;
    let repository = repository(&pool);
    repository
        .insert_audit_log(&record())
        .await
        .expect("the first row inserts");
    let error = repository
        .insert_audit_log(&record())
        .await
        .expect_err("the primary key is taken");
    assert!(
        matches!(error, AuditRepositoryError::Table(TableError::Database(_))),
        "{error:?}"
    );
    assert!(error.to_string().contains("unique"), "{error}");
    pool.close();
    fixture.close_async().await.unwrap();
}
