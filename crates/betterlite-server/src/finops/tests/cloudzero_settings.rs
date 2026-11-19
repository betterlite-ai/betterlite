use betterlite_db::finops::cloudzero_settings::CLOUDZERO_SETTINGS_PARAM_NAME;

use crate::finops::cloudzero_settings::is_cloudzero_setup;
use crate::finops::cloudzero_settings::is_cloudzero_setup_in_config;
use crate::finops::cloudzero_settings::is_cloudzero_setup_in_db;

use super::support::Database;
use super::support::callbacks;
use super::support::drop_table;
use super::support::seed_null;
use super::support::seed_value;

const STORED_SETTINGS: &str = r#"{"api_key":"sealed","connection_id":"cz-connection"}"#;

#[test]
fn config_setup_matches_only_the_cloudzero_callback_name() {
    assert!(is_cloudzero_setup_in_config(&callbacks(&["cloudzero"])));
    assert!(is_cloudzero_setup_in_config(&callbacks(&[
        "langfuse",
        "cloudzero"
    ])));
    assert!(!is_cloudzero_setup_in_config(&callbacks(&["vantage"])));
    assert!(!is_cloudzero_setup_in_config(&callbacks(&[
        "cloudzero_logger"
    ])));
    assert!(!is_cloudzero_setup_in_config(&[]));
}

#[tokio::test]
async fn db_setup_reports_a_stored_value() {
    let database = Database::provision().await;
    assert!(!is_cloudzero_setup_in_db(Some(database.client())).await);
    seed_value(&database, CLOUDZERO_SETTINGS_PARAM_NAME, STORED_SETTINGS).await;
    assert!(is_cloudzero_setup_in_db(Some(database.client())).await);
    database.close().await;
}

#[tokio::test]
async fn db_setup_reports_a_null_value_as_unconfigured() {
    let database = Database::provision().await;
    seed_null(&database, CLOUDZERO_SETTINGS_PARAM_NAME).await;
    assert!(!is_cloudzero_setup_in_db(Some(database.client())).await);
    database.close().await;
}

#[tokio::test]
async fn db_setup_reports_a_value_that_does_not_decode() {
    let database = Database::provision().await;
    seed_value(&database, CLOUDZERO_SETTINGS_PARAM_NAME, "not json").await;
    assert!(is_cloudzero_setup_in_db(Some(database.client())).await);
    database.close().await;
}

#[tokio::test]
async fn db_setup_reports_false_when_the_query_fails() {
    let database = Database::provision().await;
    drop_table(&database, "LiteLLM_Config").await;
    assert!(!is_cloudzero_setup_in_db(Some(database.client())).await);
    database.close().await;
}

#[tokio::test]
async fn db_setup_reports_false_without_a_database() {
    assert!(!is_cloudzero_setup_in_db(None).await);
}

#[tokio::test]
async fn setup_composes_the_config_and_database_sources() {
    let database = Database::provision().await;
    assert!(!is_cloudzero_setup(Some(database.client()), &callbacks(&["vantage"])).await);
    assert!(is_cloudzero_setup(None, &callbacks(&["cloudzero"])).await);
    seed_value(&database, CLOUDZERO_SETTINGS_PARAM_NAME, STORED_SETTINGS).await;
    assert!(is_cloudzero_setup(Some(database.client()), &[]).await);
    assert!(is_cloudzero_setup(Some(database.client()), &callbacks(&["langfuse"])).await);
    database.close().await;
}
