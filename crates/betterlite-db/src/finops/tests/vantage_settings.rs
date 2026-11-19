use secrecy::SecretString;
use serde_json::Number;

use crate::budget_permission_rows::JsonData;
use crate::finops::settings_row::SettingsRowError;
use crate::finops::vantage_settings::VANTAGE_API_KEY;
use crate::finops::vantage_settings::VANTAGE_INTEGRATION_TOKEN;
use crate::finops::vantage_settings::VANTAGE_SETTINGS_PARAM_NAME;
use crate::finops::vantage_settings::VantageSettingsWrite;
use crate::finops::vantage_settings::get_vantage_settings;
use crate::finops::vantage_settings::set_vantage_settings;
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

const PLAINTEXT_KEY: &str = "vantage-api-key-value";
const PLAINTEXT_TOKEN: &str = "vantage-integration-token-value";
const BASE_URL: &str = "https://api.vantage.sh";

fn settings(api_key: &str, integration_token: &str, base_url: &str) -> VantageSettingsWrite {
    VantageSettingsWrite {
        api_key: SecretString::from(api_key.to_owned()),
        integration_token: SecretString::from(integration_token.to_owned()),
        base_url: base_url.to_owned(),
    }
}

fn row_json(api_key: JsonData, integration_token: JsonData, base_url: JsonData) -> String {
    json_object(&[
        ("api_key", api_key),
        ("integration_token", integration_token),
        ("base_url", base_url),
    ])
}

#[tokio::test]
async fn set_stores_both_credentials_encrypted_and_the_base_url_plaintext() {
    let database = Database::provision().await;
    set_vantage_settings(
        Some(database.client()),
        &cipher(),
        &settings(PLAINTEXT_KEY, PLAINTEXT_TOKEN, BASE_URL),
    )
    .await
    .unwrap();
    let stored = read_row(&database, VANTAGE_SETTINGS_PARAM_NAME)
        .await
        .unwrap();
    assert_eq!(stored.kind, "string");
    let values = stored.fields();
    let Some(JsonData::String(sealed_key)) = values.get("api_key") else {
        panic!("the api key is stored as text");
    };
    assert_ne!(sealed_key, PLAINTEXT_KEY);
    assert!(!sealed_key.contains(PLAINTEXT_KEY));
    let Some(JsonData::String(sealed_token)) = values.get("integration_token") else {
        panic!("the integration token is stored as text");
    };
    assert_ne!(sealed_token, PLAINTEXT_TOKEN);
    assert!(!sealed_token.contains(PLAINTEXT_TOKEN));
    assert_eq!(
        values.get("base_url"),
        Some(&JsonData::String(BASE_URL.to_owned()))
    );
    database.close().await;
}

#[tokio::test]
async fn set_then_get_round_trips_the_settings() {
    let database = Database::provision().await;
    set_vantage_settings(
        Some(database.client()),
        &cipher(),
        &settings(PLAINTEXT_KEY, PLAINTEXT_TOKEN, BASE_URL),
    )
    .await
    .unwrap();
    let loaded = get_vantage_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(loaded.api_key.as_ref()), Some(PLAINTEXT_KEY));
    assert_eq!(
        exposed(loaded.integration_token.as_ref()),
        Some(PLAINTEXT_TOKEN)
    );
    assert_eq!(loaded.base_url.as_deref(), Some(BASE_URL));
    database.close().await;
}

#[tokio::test]
async fn set_replaces_the_settings_of_an_existing_row() {
    let database = Database::provision().await;
    set_vantage_settings(
        Some(database.client()),
        &cipher(),
        &settings("first-key", "first-token", "https://first.example"),
    )
    .await
    .unwrap();
    set_vantage_settings(
        Some(database.client()),
        &cipher(),
        &settings(PLAINTEXT_KEY, PLAINTEXT_TOKEN, BASE_URL),
    )
    .await
    .unwrap();
    let loaded = get_vantage_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(loaded.api_key.as_ref()), Some(PLAINTEXT_KEY));
    assert_eq!(
        exposed(loaded.integration_token.as_ref()),
        Some(PLAINTEXT_TOKEN)
    );
    assert_eq!(loaded.base_url.as_deref(), Some(BASE_URL));
    database.close().await;
}

#[tokio::test]
async fn get_reads_a_json_string_row() {
    let database = Database::provision().await;
    seed_string(
        &database,
        VANTAGE_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::String(seal(PLAINTEXT_TOKEN)),
            JsonData::String(BASE_URL.to_owned()),
        ),
    )
    .await;
    let loaded = get_vantage_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(loaded.api_key.as_ref()), Some(PLAINTEXT_KEY));
    assert_eq!(
        exposed(loaded.integration_token.as_ref()),
        Some(PLAINTEXT_TOKEN)
    );
    assert_eq!(loaded.base_url.as_deref(), Some(BASE_URL));
    database.close().await;
}

#[tokio::test]
async fn get_reads_a_mapping_row() {
    let database = Database::provision().await;
    seed_mapping(
        &database,
        VANTAGE_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::String(seal(PLAINTEXT_TOKEN)),
            JsonData::String(BASE_URL.to_owned()),
        ),
    )
    .await;
    let loaded = get_vantage_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(loaded.api_key.as_ref()), Some(PLAINTEXT_KEY));
    assert_eq!(loaded.base_url.as_deref(), Some(BASE_URL));
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_provider_without_settings_as_none() {
    let database = Database::provision().await;
    assert!(
        get_vantage_settings(Some(database.client()), &cipher())
            .await
            .unwrap()
            .is_none()
    );
    seed_null(&database, VANTAGE_SETTINGS_PARAM_NAME).await;
    assert!(
        get_vantage_settings(Some(database.client()), &cipher())
            .await
            .unwrap()
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_malformed_row_as_an_error() {
    let database = Database::provision().await;
    seed_string(&database, VANTAGE_SETTINGS_PARAM_NAME, "not json").await;
    let result = get_vantage_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Read {
            param: VANTAGE_SETTINGS_PARAM_NAME,
            source: ConfigError::Json(_)
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_rejects_a_row_that_is_not_a_mapping() {
    let database = Database::provision().await;
    seed_mapping(&database, VANTAGE_SETTINGS_PARAM_NAME, "[]").await;
    let result = get_vantage_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Shape {
            param: VANTAGE_SETTINGS_PARAM_NAME
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_names_the_api_key_when_another_salt_sealed_it() {
    let database = Database::provision().await;
    seed_string(
        &database,
        VANTAGE_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal_with(PLAINTEXT_KEY, "another-salt")),
            JsonData::String(seal(PLAINTEXT_TOKEN)),
            JsonData::String(BASE_URL.to_owned()),
        ),
    )
    .await;
    let result = get_vantage_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Decrypt {
            field: VANTAGE_API_KEY
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_names_the_integration_token_when_another_salt_sealed_it() {
    let database = Database::provision().await;
    seed_string(
        &database,
        VANTAGE_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::String(seal_with(PLAINTEXT_TOKEN, "another-salt")),
            JsonData::String(BASE_URL.to_owned()),
        ),
    )
    .await;
    let result = get_vantage_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Decrypt {
            field: VANTAGE_INTEGRATION_TOKEN
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_decrypt_failure_without_a_salt_key() {
    let database = Database::provision().await;
    seed_string(
        &database,
        VANTAGE_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::String(seal(PLAINTEXT_TOKEN)),
            JsonData::String(BASE_URL.to_owned()),
        ),
    )
    .await;
    let result = get_vantage_settings(Some(database.client()), &cipher_without_key()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Decrypt {
            field: VANTAGE_API_KEY
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn get_keeps_an_empty_integration_token_empty() {
    let database = Database::provision().await;
    seed_string(
        &database,
        VANTAGE_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::String(String::new()),
            JsonData::String(BASE_URL.to_owned()),
        ),
    )
    .await;
    let loaded = get_vantage_settings(Some(database.client()), &cipher())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(loaded.integration_token.as_ref()), Some(""));
    database.close().await;
}

#[tokio::test]
async fn get_rejects_a_base_url_that_is_not_a_string() {
    let database = Database::provision().await;
    seed_string(
        &database,
        VANTAGE_SETTINGS_PARAM_NAME,
        &row_json(
            JsonData::String(seal(PLAINTEXT_KEY)),
            JsonData::Null,
            JsonData::Number(Number::from(5_i32)),
        ),
    )
    .await;
    let result = get_vantage_settings(Some(database.client()), &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Field {
            param: VANTAGE_SETTINGS_PARAM_NAME,
            field: "base_url"
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn set_reports_a_missing_salt_key_without_writing() {
    let database = Database::provision().await;
    let result = set_vantage_settings(
        Some(database.client()),
        &cipher_without_key(),
        &settings(PLAINTEXT_KEY, PLAINTEXT_TOKEN, BASE_URL),
    )
    .await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Encrypt {
            field: VANTAGE_API_KEY,
            ..
        })
    ));
    assert!(
        read_row(&database, VANTAGE_SETTINGS_PARAM_NAME)
            .await
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
async fn get_reports_a_missing_database() {
    let result = get_vantage_settings(None, &cipher()).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Read {
            param: VANTAGE_SETTINGS_PARAM_NAME,
            source: ConfigError::Database(DbError::NoDbConnected)
        })
    ));
}
