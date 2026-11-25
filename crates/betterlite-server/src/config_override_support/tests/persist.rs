#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::{object, text, value};
use crate::config_override_support::persist::{
    ConfigOverrideStore, ConfigOverrideStoreError, DatabaseConfigOverrideStore,
    PersistOverrideError, persist_cyberark_config,
};
use crate::env_value_codec::decrypt_env;

struct RecordingStore {
    calls: Mutex<Vec<(String, String)>>,
    fail: bool,
}

impl RecordingStore {
    const fn new(fail: bool) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail,
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ConfigOverrideStore for RecordingStore {
    async fn upsert_config_override(
        &self,
        config_type: &str,
        config_value: &str,
    ) -> Result<(), ConfigOverrideStoreError> {
        if self.fail {
            return Err(ConfigOverrideStoreError::Table(TableError::MissingRow));
        }
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((config_type.to_owned(), config_value.to_owned()));
        Ok(())
    }
}

fn cipher(signing_key: Option<&str>) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: signing_key.map(SecretString::from),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

#[tokio::test]
async fn encrypts_every_value_and_upserts_the_cyberark_row() {
    let store = RecordingStore::new(false);
    let cipher = cipher(Some("test-signing-key"));
    let payload = persist_cyberark_config(
        &store,
        &cipher,
        &object(
            r#"{
                "cyberark_api_base": "https://conjur.example.com",
                "cyberark_api_key": "hunter2",
                "refresh_interval": ""
            }"#,
        ),
    )
    .await
    .unwrap();
    assert_eq!(payload.len(), 3);
    assert_ne!(payload.get("cyberark_api_key"), Some(&text("hunter2")));
    assert_eq!(
        decrypt_env(&payload, &cipher),
        object(
            r#"{
                "cyberark_api_base": "https://conjur.example.com",
                "cyberark_api_key": "hunter2",
                "refresh_interval": ""
            }"#
        )
    );
    let calls = store.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "cyberark");
    assert_eq!(
        serde_json::from_str::<JsonValue>(&calls[0].1).unwrap(),
        JsonValue::Object(payload)
    );
}

#[tokio::test]
async fn leaves_non_string_values_for_the_column() {
    let store = RecordingStore::new(false);
    let cipher = cipher(Some("test-signing-key"));
    let payload = persist_cyberark_config(
        &store,
        &cipher,
        &object(r#"{"cyberark_account": null, "refresh_interval": 60}"#),
    )
    .await
    .unwrap();
    assert_eq!(payload.get("cyberark_account"), Some(&JsonValue::Null));
    assert_eq!(payload.get("refresh_interval"), Some(&value("60")));
}

#[tokio::test]
async fn a_missing_signing_key_fails_before_the_store() {
    let store = RecordingStore::new(false);
    let error = persist_cyberark_config(
        &store,
        &cipher(None),
        &object(r#"{"cyberark_api_key": "hunter2"}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, PersistOverrideError::Encrypt(_)));
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn a_store_failure_is_reported() {
    let store = RecordingStore::new(true);
    let error = persist_cyberark_config(
        &store,
        &cipher(Some("test-signing-key")),
        &object(r#"{"cyberark_account": "acct"}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, PersistOverrideError::Store(_)));
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ConfigOverrides" (
    config_type TEXT PRIMARY KEY,
    config_value JSONB NOT NULL,
    created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP(3) NOT NULL
);
"#;

const SELECT_SQL: &str =
    r#"SELECT config_value::text FROM "LiteLLM_ConfigOverrides" WHERE config_type = $1"#;

const SEED_UPDATED_AT_SQL: &str = r#"UPDATE "LiteLLM_ConfigOverrides" SET updated_at = TIMESTAMP '2000-01-01 00:00:00' WHERE config_type = $1"#;

const SELECT_UPDATED_AT_SQL: &str =
    r#"SELECT updated_at::text FROM "LiteLLM_ConfigOverrides" WHERE config_type = $1"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["config-override-support-tests"])
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

fn repository(pool: &Pool) -> DatabaseConfigOverrideStore {
    DatabaseConfigOverrideStore {
        store: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
        availability: Availability::default(),
    }
}

async fn stored(pool: &Pool) -> JsonValue {
    let client = pool.get().await.unwrap();
    let row = client.query_one(SELECT_SQL, &[&"cyberark"]).await.unwrap();
    let text: String = row.get(0);
    serde_json::from_str(&text).unwrap()
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn the_database_store_creates_and_updates_the_cyberark_row() {
    let (fixture, pool) = database().await;
    let store = repository(&pool);
    store
        .upsert_config_override("cyberark", r#"{"cyberark_account": "first"}"#)
        .await
        .unwrap();
    assert_eq!(
        stored(&pool).await,
        value(r#"{"cyberark_account": "first"}"#)
    );
    let client = pool.get().await.unwrap();
    let seeded = client
        .execute(SEED_UPDATED_AT_SQL, &[&"cyberark"])
        .await
        .unwrap();
    assert_eq!(seeded, 1);
    drop(client);
    store
        .upsert_config_override("cyberark", r#"{"cyberark_account": "second"}"#)
        .await
        .unwrap();
    assert_eq!(
        stored(&pool).await,
        value(r#"{"cyberark_account": "second"}"#)
    );
    let reader = pool.get().await.unwrap();
    let row = reader
        .query_one(SELECT_UPDATED_AT_SQL, &[&"cyberark"])
        .await
        .unwrap();
    let updated_at: String = row.get(0);
    assert_ne!(updated_at, "2000-01-01 00:00:00");
    drop(reader);
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_database_store_reports_a_closed_pool() {
    let (fixture, pool) = database().await;
    let store = repository(&pool);
    pool.close();
    let error = store
        .upsert_config_override("cyberark", r#"{"cyberark_account": "first"}"#)
        .await
        .unwrap_err();
    assert!(matches!(error, ConfigOverrideStoreError::Table(_)));
    close(fixture, pool).await;
}
