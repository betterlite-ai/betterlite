use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::config::{ConfigDecodeError, ConfigError};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};

use super::support::object;
use crate::coordination_redis_settings::store::{
    DatabaseGeneralSettingsStore, GeneralSettingsStore, SettingsError, current_settings,
    persisted_settings,
};

struct FakeStore {
    row: Option<JsonData>,
    fails: bool,
}

#[async_trait]
impl GeneralSettingsStore for FakeStore {
    async fn general_settings(&self) -> Result<Option<JsonData>, ConfigError> {
        if self.fails {
            return Err(ConfigError::Json(ConfigDecodeError::TooLarge { len: 1 }));
        }
        Ok(self.row.clone())
    }
}

fn row(text: &str) -> JsonData {
    serde_json::from_str(text).expect("a JSON row")
}

fn store(text: &str) -> FakeStore {
    FakeStore {
        row: Some(row(text)),
        fails: false,
    }
}

fn miss() -> FakeStore {
    FakeStore {
        row: None,
        fails: false,
    }
}

#[tokio::test]
async fn without_a_store_there_is_no_persisted_block() {
    assert_eq!(persisted_settings(None).await.unwrap(), None);
    assert_eq!(
        current_settings(
            None,
            &object(r#"{"general_settings": {"coordination_redis": {"host": "file-host"}}}"#)
        )
        .await
        .unwrap(),
        Some(object(r#"{"host": "file-host"}"#)),
    );
}

#[tokio::test]
async fn a_missing_or_null_row_is_an_empty_section() {
    assert_eq!(persisted_settings(Some(&miss())).await.unwrap(), None);
    assert_eq!(
        persisted_settings(Some(&store("null"))).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn an_empty_saved_block_is_still_a_block() {
    let saved = store(r#"{"coordination_redis": {}, "master_key": "sk-x"}"#);
    assert_eq!(
        persisted_settings(Some(&saved)).await.unwrap(),
        Some(object("{}"))
    );
}

#[tokio::test]
async fn a_non_dictionary_coordination_redis_is_no_block() {
    let saved = store(r#"{"coordination_redis": "os.environ/BLOCK"}"#);
    assert_eq!(persisted_settings(Some(&saved)).await.unwrap(), None);
}

#[tokio::test]
async fn a_non_object_row_is_an_error() {
    let saved = store(r#"["coordination_redis"]"#);
    let error = persisted_settings(Some(&saved))
        .await
        .expect_err("a non-object row is an error");
    assert!(matches!(error, SettingsError::GeneralSettingsNotAnObject));
}

#[tokio::test]
async fn a_failed_row_read_is_an_error() {
    let failing = FakeStore {
        row: None,
        fails: true,
    };
    let error = persisted_settings(Some(&failing))
        .await
        .expect_err("a failed read is an error");
    assert!(matches!(error, SettingsError::Config(_)));
}

#[tokio::test]
async fn a_saved_block_wins_over_the_file_state_even_when_empty() {
    let saved = store(r#"{"coordination_redis": {}}"#);
    let state = object(r#"{"general_settings": {"coordination_redis": {"host": "file-host"}}}"#);
    assert_eq!(
        current_settings(Some(&saved), &state).await.unwrap(),
        Some(object("{}")),
    );
}

#[tokio::test]
async fn the_file_state_fills_in_without_a_saved_block() {
    let missing = store(r#"{"master_key": "sk-x"}"#);
    let state = object(r#"{"general_settings": {"coordination_redis": {"host": "file-host"}}}"#);
    assert_eq!(
        current_settings(Some(&missing), &state).await.unwrap(),
        Some(object(r#"{"host": "file-host"}"#)),
    );
}

#[rstest]
#[case::no_general_settings("{}")]
#[case::null_general_settings(r#"{"general_settings": null}"#)]
#[case::list_general_settings(r#"{"general_settings": ["coordination_redis"]}"#)]
#[case::text_block(r#"{"general_settings": {"coordination_redis": "text"}}"#)]
#[tokio::test]
async fn a_state_without_a_general_settings_mapping_has_no_block(#[case] state: &str) {
    assert_eq!(current_settings(None, &object(state)).await.unwrap(), None);
}

#[tokio::test]
async fn a_non_object_config_state_is_an_error() {
    let state = object(r#"["general_settings"]"#);
    let error = current_settings(None, &state)
        .await
        .expect_err("a non-object state is an error");
    assert!(matches!(error, SettingsError::ConfigStateNotAnObject));
}

const CONFIG_TABLE: &str = r#"CREATE TABLE "LiteLLM_Config" (
    param_name TEXT NOT NULL,
    param_value JSONB,
    CONSTRAINT "LiteLLM_Config_pkey" PRIMARY KEY (param_name)
);
INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES
    ('general_settings', '{"coordination_redis": {"host": "db-host", "port": 6380, "password": "os.environ/REDIS_PASSWORD"}, "master_key": "sk-x"}'::JSONB);"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

#[tokio::test]
async fn a_database_row_reaches_the_persisted_block() {
    let url = Environment::parse_from(["coordination-redis-settings-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![CONFIG_TABLE.to_owned()])
            .resolve()
            .await
            .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(2)
        .build()
        .unwrap();
    let database = DatabaseGeneralSettingsStore {
        store: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
        availability: Availability::default(),
    };
    assert_eq!(
        persisted_settings(Some(&database)).await.unwrap(),
        Some(object(
            r#"{"host": "db-host", "port": 6380, "password": "os.environ/REDIS_PASSWORD"}"#,
        )),
    );
    pool.close();
    fixture.close_async().await.unwrap();
}
