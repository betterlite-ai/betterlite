use secrecy::SecretString;
use serde_json::Number;

use crate::budget_permission_rows::JsonData;
use crate::finops::cloudzero_settings::CLOUDZERO_API_KEY;
use crate::finops::cloudzero_settings::CLOUDZERO_SETTINGS_PARAM_NAME;
use crate::finops::cloudzero_settings::CloudZeroSettingsWrite;
use crate::finops::cloudzero_settings::get_cloudzero_settings;
use crate::finops::cloudzero_settings::set_cloudzero_settings;
use crate::finops::settings_row::SettingsRowError;
use crate::postgres_store::errors::DbError;
use crate::resource_repository::config::ConfigError;

use super::support::Database;
use super::support::cipher;
use super::support::cipher_without_key;
use super::support::exposed;
use super::support::json_object;
use super::support::read_row;
use super::support::seal;
use super::support::seal_with;
use super::support::seed_mapping;
use super::support::seed_null;
use super::support::seed_string;

const PLAINTEXT_KEY: &str = "cz-api-key-value";
const CONNECTION_ID: &str = "cz-connection-1";
const TIMEZONE: &str = "Europe/Berlin";

fn settings(api_key: &str, connection_id: &str, timezone: &str) -> CloudZeroSettingsWrite {
    CloudZeroSettingsWrite {
        api_key: SecretString::from(api_key.to_owned()),
        connection_id: connection_id.to_owned(),
        timezone: timezone.to_owned(),
    }
}

fn row_json(api_key: JsonData, connection_id: JsonData, timezone: JsonData) -> String {
    json_object(&[
        ("api_key", api_key),
        ("connection_id", connection_id),
        ("timezone", timezone),
    ])
}

#[tokio::test]
async fn set_stores_the_api_key_encrypted_and_the_rest_plaintext() {
    let database = Database::provision().await;
    set_cloudzero_settings(
        Some(database.client()),
        &cipher(),
        &settings(PLAINTEXT_KEY, CONNECTION_ID, TIMEZONE),
    )
    .await
    .unwrap();
    let stored = read_row(&database, CLOUDZERO_SETTINGS_PARAM_NAME)
        .await
        .unwrap();
    assert_eq!(stored.kind, "string");
    let values = stored.fields();
    let Some(JsonData::String(sealed_key)) = values.get("api_key") else {
        panic!("the api key is stored as text");
    };
    assert_ne!(sealed_key, PLAINTEXT_KEY);
    assert!(!sealed_key.contains(PLAINTEXT_KEY));
    assert_eq!(
        values.get("connection_id"),
        Some(&JsonData::String(CONNECTION_ID.to_owned()))
    );
    assert_eq!(
        values.get("timezone"),
        Some(&JsonData::String(TIMEZONE.to_owned()))
    );
    database.close().await;
}

#[tokio::test]
async fn set_then_get_round_trips_the_settings() {
    let database = Database::provision().await;
    set_cloudzero_settings(
        Some(database.client()),
        &cipher(),
        &settings(PLAINTEXT_KEY, CONNECTION_ID, TIMEZONE),
    )
    .await
    .unwrap();
    let loaded = get_cloudzero_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(&loaded.api_key), Some(PLAINTEXT_KEY));
    assert_eq!(loaded.connection_id.as_deref(), Some(CONNECTION_ID));
    assert_eq!(loaded.timezone.as_deref(), Some(TIMEZONE));
    database.close().await;
}

#[tokio::test]
async fn set_replaces_the_settings_of_an_existing_row() {
    let database = Database::provision().await;
    set_cloudzero_settings(
        Some(database.client()),
        &cipher(),
        &settings("first-key", "first-connection", "UTC"),
    )
    .await
    .unwrap();
    set_cloudzero_settings(
        Some(database.client()),
        &cipher(),
        &settings(PLAINTEXT_KEY, CONNECTION_ID, TIMEZONE),
    )
    .await
    .unwrap();
    let loaded = get_cloudzero_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(&loaded.api_key), Some(PLAINTEXT_KEY));
    assert_eq!(loaded.connection_id.as_deref(), Some(CONNECTION_ID));
    assert_eq!(loaded.timezone.as_deref(), Some(TIMEZONE));
    database.close().await;
}

#[tokio::test]
async fn get_reads_a_json_string_row() {
    let database = Database::provision().await;
    seed_string(
        &database,
        CLOUDZERO_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::String(CONNECTION_ID.to_owned()),
            JsonData::String(TIMEZONE.to_owned()),
        ),
    )
    .await;
    let loaded = get_cloudzero_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(&loaded.api_key), Some(PLAINTEXT_KEY));
    assert_eq!(loaded.connection_id.as_deref(), Some(CONNECTION_ID));
    assert_eq!(loaded.timezone.as_deref(), Some(TIMEZONE));
    database.close().await;
}

#[tokio::test]
async fn get_reads_a_mapping_row() {
    let database = Database::provision().await;
    seed_mapping(
        &database,
        CLOUDZERO_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::String(CONNECTION_ID.to_owned()),
            JsonData::String(TIMEZONE.to_owned()),
        ),
    )
    .await;
    let loaded = get_cloudzero_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(&loaded.api_key), Some(PLAINTEXT_KEY));
    assert_eq!(loaded.connection_id.as_deref(), Some(CONNECTION_ID));
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_provider_without_settings_as_none() {
    let database = Database::provision().await;
    assert!(
        get_cloudzero_settings(Some(database.client()), &cipher())
            .await
            .unwrap()
            .is_none()
    );
    seed_null(&database, CLOUDZERO_SETTINGS_PARAM_NAME).await;
    assert!(
        get_cloudzero_settings(Some(database.client()), &cipher())
            .await
            .unwrap()
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_malformed_row_as_an_error() {
    let database = Database::provision().await;
    seed_string(&database, CLOUDZERO_SETTINGS_PARAM_NAME, "not json").await;
    let result = get_cloudzero_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Read {
            param: CLOUDZERO_SETTINGS_PARAM_NAME,
            source: ConfigError::Json(_)
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_rejects_a_row_that_is_not_a_mapping() {
    let database = Database::provision().await;
    seed_mapping(&database, CLOUDZERO_SETTINGS_PARAM_NAME, "[]").await;
    let result = get_cloudzero_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Shape {
            param: CLOUDZERO_SETTINGS_PARAM_NAME
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_decrypt_failure_for_another_salt() {
    let database = Database::provision().await;
    seed_string(
        &database,
        CLOUDZERO_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal_with(PLAINTEXT_KEY, "another-salt")),
            JsonData::String(CONNECTION_ID.to_owned()),
            JsonData::String(TIMEZONE.to_owned()),
        ),
    )
    .await;
    let result = get_cloudzero_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Decrypt {
            field: CLOUDZERO_API_KEY
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_decrypt_failure_without_a_salt_key() {
    let database = Database::provision().await;
    seed_string(
        &database,
        CLOUDZERO_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::Null,
            JsonData::Null,
        ),
    )
    .await;
    let result = get_cloudzero_settings(Some(database.client()), &cipher_without_key()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Decrypt {
            field: CLOUDZERO_API_KEY
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_keeps_an_empty_api_key_empty() {
    let database = Database::provision().await;
    seed_string(
        &database,
        CLOUDZERO_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(String::new()),
            JsonData::String(CONNECTION_ID.to_owned()),
            JsonData::String(TIMEZONE.to_owned()),
        ),
    )
    .await;
    let loaded = get_cloudzero_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(&loaded.api_key), Some(""));
    database.close().await;
}

#[tokio::test]
async fn get_rejects_a_connection_id_that_is_not_a_string() {
    let database = Database::provision().await;
    seed_string(
        &database,
        CLOUDZERO_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::Null,
            JsonData::Number(Number::from(5)),
            JsonData::String(TIMEZONE.to_owned()),
        ),
    )
    .await;
    let result = get_cloudzero_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Field {
            param: CLOUDZERO_SETTINGS_PARAM_NAME,
            field: "connection_id"
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn set_reports_a_missing_salt_key_without_writing() {
    let database = Database::provision().await;
    let result = set_cloudzero_settings(
        Some(database.client()),
        &cipher_without_key(),
        &settings(PLAINTEXT_KEY, CONNECTION_ID, TIMEZONE),
    )
    .await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Encrypt {
            field: CLOUDZERO_API_KEY,
            ..
        })
    ));
    assert!(
        read_row(&database, CLOUDZERO_SETTINGS_PARAM_NAME)
            .await
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_missing_database() {
    let result = get_cloudzero_settings(None, &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Read {
            param: CLOUDZERO_SETTINGS_PARAM_NAME,
            source: ConfigError::Database(DbError::NoDbConnected)
        })
    ));
}
