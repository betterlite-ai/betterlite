use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use crate::budget_permission_rows::JsonData;
use crate::finops::settings_row::SettingsRowError;
use crate::finops::settings_row::plain_field;
use crate::finops::settings_row::read_settings_row;
use crate::finops::settings_row::seal_secret;
use crate::finops::settings_row::secret_field;
use crate::finops::settings_row::write_settings_row;
use crate::finops::settings_row_presence::settings_row_present;
use crate::postgres_store::errors::DbError;
use crate::resource_repository::config::ConfigError;

use super::support::Database;
use super::support::cipher;
use super::support::cipher_with_key;
use super::support::cipher_without_key;
use super::support::exposed;
use super::support::fields;
use super::support::json_object;
use super::support::read_row;
use super::support::seal;
use super::support::seal_with;
use super::support::seed_mapping;
use super::support::seed_null;
use super::support::seed_string;

const PARAM: &str = "settings_row_under_test";
const OTHER_PARAM: &str = "other_settings_row";
const KEY: &str = "settings_row_test_key";

#[test]
fn plain_field_maps_an_absent_or_null_value_to_none() {
    let values = fields(&[
        ("connection_id", JsonData::String("conn-1".to_owned())),
        ("timezone", JsonData::Null),
    ]);
    assert_eq!(
        plain_field(&values, PARAM, "connection_id").unwrap(),
        Some("conn-1".to_owned())
    );
    assert_eq!(plain_field(&values, PARAM, "timezone").unwrap(), None);
    assert_eq!(plain_field(&values, PARAM, "absent").unwrap(), None);
}

#[rstest]
#[case::boolean(JsonData::Bool(true))]
#[case::number(JsonData::Number(Number::from(5)))]
#[case::array(JsonData::Array(Vec::new()))]
#[case::object(JsonData::Object(BTreeMap::new()))]
fn plain_field_rejects_a_value_that_is_not_a_string(#[case] value: JsonData) {
    let values = fields(&[("connection_id", value)]);
    let result = plain_field(&values, PARAM, "connection_id");
    assert!(matches!(
        result,
        Err(SettingsRowError::Field {
            param: PARAM,
            field: "connection_id"
        })
    ));
}

#[test]
fn secret_field_maps_an_absent_or_null_credential_to_none() {
    let values = fields(&[("api_key", JsonData::Null)]);
    assert!(
        secret_field(&values, PARAM, "api_key", KEY, &cipher())
            .unwrap()
            .is_none()
    );
    assert!(
        secret_field(&values, PARAM, "absent", KEY, &cipher())
            .unwrap()
            .is_none()
    );
}

#[test]
fn secret_field_rejects_a_credential_that_is_not_a_string() {
    let values = fields(&[("api_key", JsonData::Number(Number::from(5)))]);
    let result = secret_field(&values, PARAM, "api_key", KEY, &cipher());
    assert!(matches!(
        result,
        Err(SettingsRowError::Field {
            param: PARAM,
            field: "api_key"
        })
    ));
}

#[test]
fn secret_field_keeps_an_empty_credential_empty_without_a_salt_key() {
    let values = fields(&[("api_key", JsonData::String(String::new()))]);
    let secret = secret_field(&values, PARAM, "api_key", KEY, &cipher_without_key()).unwrap();
    assert_eq!(exposed(&secret), Some(""));
}

#[test]
fn secret_field_decrypts_a_credential_sealed_with_the_salt_key() {
    let sealed = seal("cz-api-key-value");
    assert_ne!(sealed, "cz-api-key-value");
    let values = fields(&[("api_key", JsonData::String(sealed))]);
    let secret = secret_field(&values, PARAM, "api_key", KEY, &cipher()).unwrap();
    assert_eq!(exposed(&secret), Some("cz-api-key-value"));
}

#[test]
fn secret_field_names_the_key_when_another_salt_sealed_the_credential() {
    let values = fields(&[(
        "api_key",
        JsonData::String(seal_with("cz-api-key-value", "another-salt")),
    )]);
    let result = secret_field(&values, PARAM, "api_key", KEY, &cipher());
    assert!(matches!(
        result,
        Err(SettingsRowError::Decrypt { field: KEY })
    ));
}

#[test]
fn secret_field_names_the_key_when_the_salt_key_is_missing() {
    let values = fields(&[("api_key", JsonData::String(seal("cz-api-key-value")))]);
    let result = secret_field(&values, PARAM, "api_key", KEY, &cipher_without_key());
    assert!(matches!(
        result,
        Err(SettingsRowError::Decrypt { field: KEY })
    ));
}

#[test]
fn seal_secret_hides_the_plaintext_and_the_same_salt_opens_it() {
    let plaintext = SecretString::from("cz-api-key-value".to_owned());
    let sealed = seal_secret(&cipher(), KEY, &plaintext).unwrap();
    assert_ne!(sealed, "cz-api-key-value");
    assert!(!sealed.contains("cz-api-key-value"));
    let values = fields(&[("api_key", JsonData::String(sealed))]);
    let opened = secret_field(&values, PARAM, "api_key", KEY, &cipher()).unwrap();
    assert_eq!(exposed(&opened), Some("cz-api-key-value"));
    let other = secret_field(
        &values,
        PARAM,
        "api_key",
        KEY,
        &cipher_with_key("another-salt"),
    );
    assert!(matches!(
        other,
        Err(SettingsRowError::Decrypt { field: KEY })
    ));
}

#[test]
fn seal_secret_reports_a_missing_salt_key() {
    let plaintext = SecretString::from("cz-api-key-value".to_owned());
    let result = seal_secret(&cipher_without_key(), KEY, &plaintext);
    assert!(matches!(
        result,
        Err(SettingsRowError::Encrypt { field: KEY, .. })
    ));
}

#[tokio::test]
async fn read_settings_row_decodes_a_json_string_row() {
    let database = Database::provision().await;
    seed_string(
        &database,
        PARAM,
        &json_object(&[("connection_id", JsonData::String("conn-1".to_owned()))]),
    )
    .await;
    let row = read_settings_row(Some(database.client()), PARAM)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.get("connection_id"),
        Some(&JsonData::String("conn-1".to_owned()))
    );
    database.close().await;
}

#[tokio::test]
async fn read_settings_row_reads_a_mapping_row() {
    let database = Database::provision().await;
    seed_mapping(
        &database,
        PARAM,
        &json_object(&[("timezone", JsonData::String("UTC".to_owned()))]),
    )
    .await;
    let row = read_settings_row(Some(database.client()), PARAM)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.get("timezone"),
        Some(&JsonData::String("UTC".to_owned()))
    );
    database.close().await;
}

#[tokio::test]
async fn read_settings_row_maps_an_absent_or_null_row_to_none() {
    let database = Database::provision().await;
    assert!(
        read_settings_row(Some(database.client()), PARAM)
            .await
            .unwrap()
            .is_none()
    );
    seed_null(&database, PARAM).await;
    assert!(
        read_settings_row(Some(database.client()), PARAM)
            .await
            .unwrap()
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
async fn read_settings_row_rejects_a_malformed_json_string() {
    let database = Database::provision().await;
    seed_string(&database, PARAM, "not json").await;
    let result = read_settings_row(Some(database.client()), PARAM).await;
    assert!(matches!(
        result,
        Err(SettingsRowError::Read {
            param: PARAM,
            source: ConfigError::Json(_)
        })
    ));
    database.close().await;
}

#[tokio::test]
async fn read_settings_row_rejects_a_value_that_is_not_a_mapping() {
    let database = Database::provision().await;
    seed_mapping(&database, PARAM, "[]").await;
    let array_row = read_settings_row(Some(database.client()), PARAM).await;
    assert!(matches!(
        array_row,
        Err(SettingsRowError::Shape { param: PARAM })
    ));
    seed_string(&database, OTHER_PARAM, "5").await;
    let number_row = read_settings_row(Some(database.client()), OTHER_PARAM).await;
    assert!(matches!(
        number_row,
        Err(SettingsRowError::Shape { param: OTHER_PARAM })
    ));
    database.close().await;
}

#[tokio::test]
async fn settings_row_present_counts_a_row_that_holds_a_value() {
    let database = Database::provision().await;
    assert!(
        !settings_row_present(Some(database.client()), PARAM)
            .await
            .unwrap()
    );
    seed_string(
        &database,
        PARAM,
        &json_object(&[("timezone", JsonData::String("UTC".to_owned()))]),
    )
    .await;
    assert!(
        settings_row_present(Some(database.client()), PARAM)
            .await
            .unwrap()
    );
    database.close().await;
}

#[tokio::test]
async fn settings_row_present_ignores_a_null_value() {
    let database = Database::provision().await;
    seed_null(&database, PARAM).await;
    assert!(
        !settings_row_present(Some(database.client()), PARAM)
            .await
            .unwrap()
    );
    database.close().await;
}

#[tokio::test]
async fn settings_row_present_counts_a_value_that_does_not_decode() {
    let database = Database::provision().await;
    seed_string(&database, PARAM, "not json").await;
    assert!(
        settings_row_present(Some(database.client()), PARAM)
            .await
            .unwrap()
    );
    database.close().await;
}

#[tokio::test]
async fn settings_row_calls_report_a_missing_database() {
    assert!(matches!(
        read_settings_row(None, PARAM).await,
        Err(SettingsRowError::Read {
            param: PARAM,
            source: ConfigError::Database(DbError::NoDbConnected)
        })
    ));
    assert!(matches!(
        settings_row_present(None, PARAM).await,
        Err(SettingsRowError::Read { param: PARAM, .. })
    ));
    assert!(matches!(
        write_settings_row(None, PARAM, BTreeMap::new()).await,
        Err(SettingsRowError::Write { param: PARAM, .. })
    ));
}

#[tokio::test]
async fn write_settings_row_stores_the_fields_as_a_json_string_and_replaces_them() {
    let database = Database::provision().await;
    let first = fields(&[
        ("connection_id", JsonData::String("conn-1".to_owned())),
        ("timezone", JsonData::Null),
    ]);
    write_settings_row(Some(database.client()), PARAM, first)
        .await
        .unwrap();
    let stored = read_row(&database, PARAM).await.unwrap();
    assert_eq!(stored.kind, "string");
    let values = stored.fields();
    assert_eq!(
        values.get("connection_id"),
        Some(&JsonData::String("conn-1".to_owned()))
    );
    assert_eq!(values.get("timezone"), Some(&JsonData::Null));
    let second = fields(&[("connection_id", JsonData::String("conn-2".to_owned()))]);
    write_settings_row(Some(database.client()), PARAM, second)
        .await
        .unwrap();
    let replaced = read_row(&database, PARAM).await.unwrap();
    let values = replaced.fields();
    assert_eq!(
        values.get("connection_id"),
        Some(&JsonData::String("conn-2".to_owned()))
    );
    assert_eq!(values.get("timezone"), None);
    database.close().await;
}
