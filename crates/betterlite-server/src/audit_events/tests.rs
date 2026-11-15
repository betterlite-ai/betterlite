#![cfg(test)]

mod callbacks;
mod corpus;
mod identity;
mod records;
mod sink;
mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::audit_events::Settings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::backend;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, FixedOffset, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use self::support::{
    CollectingSpawner, FixedClock, RecordingCallback, SAMPLE_TIMESTAMP, TestRegistry, instance,
    parsed, recorder,
};
use crate::audit_events::callbacks::{
    AuditCallbackConfig, AuditCallbackDispatcher, AuditLogCallback,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::record::{AuditLogRecord, payload_text};
use crate::audit_events::settings::{AuditSettings as _, ConfiguredAuditSettings};
use crate::audit_events::sink::{
    AuditClock as _, AuditEventSink, AuditLogRepository, AuditRepositoryError, ObjectAuditLog,
    SystemAuditClock,
};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_AuditLog" (
    id TEXT PRIMARY KEY,
    updated_at TIMESTAMPTZ NOT NULL,
    changed_by TEXT,
    changed_by_api_key TEXT,
    action TEXT NOT NULL,
    table_name TEXT,
    object_id TEXT,
    before_value JSONB,
    updated_values JSONB
)
"#;

const INSERT: &str = r#"
INSERT INTO "LiteLLM_AuditLog"
    (id, updated_at, changed_by, changed_by_api_key, action, table_name, object_id, before_value, updated_values)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::jsonb, $9::text::jsonb)
"#;

const SELECT: &str = r#"
SELECT updated_at, changed_by, changed_by_api_key, action, table_name, object_id,
       before_value::text, updated_values::text
FROM "LiteLLM_AuditLog" WHERE id = $1
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct PostgresAuditRepository {
    pool: Pool,
}

#[async_trait]
impl AuditLogRepository for PostgresAuditRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|_error| AuditRepositoryError::Unavailable)?;
        let before = record.before_value.as_ref().map(payload_text);
        let updated = record.updated_values.as_ref().map(payload_text);
        let action = record.action.as_str();
        let table_name = record.table_name.as_str();
        let api_key = record
            .changed_by_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret);
        let values: Vec<&(dyn ToSql + Sync)> = vec![
            &record.id,
            &record.updated_at,
            &record.changed_by,
            &api_key,
            &action,
            &table_name,
            &record.object_id,
            &before,
            &updated,
        ];
        client
            .execute(INSERT, &values)
            .await
            .map_err(|source| AuditRepositoryError::Table(TableError::Database(backend(source))))?;
        Ok(())
    }
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["audit-events-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(SCHEMA).await.unwrap();
    drop(client);
    (fixture, pool)
}

async fn log_team_update(sink: &AuditEventSink) {
    let caller = CallerIdentity {
        user_id: Some("admin".to_owned()),
        api_key: Some(SecretString::from("sk-key".to_owned())),
        metadata: None,
        team_metadata: None,
    };
    sink.create_object_audit_log(ObjectAuditLog {
        object_id: "team-1",
        action: AuditAction::Updated,
        litellm_changed_by: None,
        caller: &caller,
        litellm_proxy_admin_name: None,
        table_name: AuditTable::Team,
        before_value: Some(r#"{"api_key": "sk-1234567890", "name": "team"}"#),
        after_value: Some(r#"{"api_key": "sk-1234567890"}"#),
    })
    .await;
}

async fn enabled_sink(pool: &Pool) -> (Arc<RecordingCallback>, DateTime<FixedOffset>) {
    let recorder = Arc::new(RecordingCallback {
        label: "recorder".to_owned(),
        fails: false,
        delivered: Mutex::new(Vec::new()),
        order: Arc::new(Mutex::new(Vec::new())),
    });
    let callback: Arc<dyn AuditLogCallback> = Arc::clone(&recorder);
    let registry = Arc::new(TestRegistry {
        instances: Mutex::new(BTreeMap::new()),
        resolves: Mutex::new(Vec::new()),
        s3_calls: Mutex::new(Vec::new()),
        s3_logger: Arc::clone(&recorder),
        s3_fails: false,
    });
    let spawner = Arc::new(CollectingSpawner::default());
    let now = DateTime::parse_from_rfc3339(SAMPLE_TIMESTAMP).unwrap();
    let settings = ConfiguredAuditSettings {
        config: Settings::default(),
        callbacks: vec![AuditCallbackConfig::Instance(callback)],
    };
    let sink = AuditEventSink {
        settings: Arc::new(settings),
        callbacks: Arc::new(AuditCallbackDispatcher::new(registry, Arc::clone(&spawner))),
        repository: Arc::new(PostgresAuditRepository { pool: pool.clone() }),
        clock: Arc::new(FixedClock { now }),
        new_id: Arc::new(|| "audit-id-1".to_owned()),
    };
    log_team_update(&sink).await;
    spawner.run_all().await;
    (recorder, now)
}

fn masked(text: Option<&str>) -> JsonValue {
    serde_json::from_str(text.unwrap()).unwrap()
}

#[test]
fn audit_configured_settings_forward_the_policy() {
    let disabled = ConfiguredAuditSettings {
        config: Settings {
            store_audit_logs: Some(false),
            store_audit_logs_env: None,
            audit_log_callbacks: Vec::new(),
            s3_audit_callback_params: None,
        },
        callbacks: Vec::new(),
    };
    assert!(!disabled.is_audit_logging_enabled(None));
    assert!(!disabled.is_audit_logging_enabled(Some(false)));
    let enabled = ConfiguredAuditSettings {
        config: Settings::default(),
        callbacks: Vec::new(),
    };
    assert!(enabled.is_audit_logging_enabled(None));
    assert!(enabled.is_audit_logging_enabled(Some(true)));
}

#[test]
fn audit_configured_settings_expose_the_callbacks() {
    let callback = recorder("configured", false);
    let settings = ConfiguredAuditSettings {
        config: Settings {
            store_audit_logs: None,
            store_audit_logs_env: None,
            audit_log_callbacks: Vec::new(),
            s3_audit_callback_params: Some(parsed(r#"{"bucket_name": "audit-bucket"}"#)),
        },
        callbacks: vec![instance(callback)],
    };
    assert_eq!(settings.audit_log_callbacks().len(), 1);
    assert_eq!(
        settings.s3_audit_callback_params().cloned(),
        Some(parsed(r#"{"bucket_name": "audit-bucket"}"#))
    );
}

#[test]
fn audit_system_clock_is_current() {
    let before = Utc::now().fixed_offset();
    let now = SystemAuditClock.now();
    assert!((now - before).num_seconds().abs() < 60);
}

#[tokio::test]
async fn audit_enabled_without_license() {
    let (fixture, pool) = database().await;
    let (recorder, now) = enabled_sink(&pool).await;
    let delivered = recorder.delivered.lock().unwrap().clone();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].updated_at, SAMPLE_TIMESTAMP);
    assert_eq!(delivered[0].changed_by, "admin");
    assert_eq!(delivered[0].hashed_changed_by_api_key, "sk-key");
    let client = pool.get().await.unwrap();
    let row = client.query_one(SELECT, &[&"audit-id-1"]).await.unwrap();
    assert_eq!(row.get::<_, DateTime<FixedOffset>>(0), now);
    assert_eq!(row.get::<_, String>(1), "admin");
    assert_eq!(row.get::<_, String>(2), "sk-key");
    assert_eq!(row.get::<_, String>(3), "updated");
    assert_eq!(row.get::<_, String>(4), "LiteLLM_TeamTable");
    assert_eq!(row.get::<_, String>(5), "team-1");
    assert_eq!(
        masked(row.get::<_, Option<String>>(6).as_deref()),
        parsed(r#"{"api_key": "sk-1*****7890", "name": "team"}"#)
    );
    assert_eq!(
        masked(row.get::<_, Option<String>>(7).as_deref()),
        parsed(r#"{"api_key": "sk-1*****7890"}"#)
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}
