use std::sync::Arc;

use betterlite_db::finops::vantage_settings::VANTAGE_SETTINGS_PARAM_NAME;

use crate::finops::focus::export_schedule::ScheduleTrigger;
use crate::finops::vantage::export::VantageExportLogger;
use crate::finops::vantage::store::VantageCallbackList;
use crate::finops::vantage::store::VantageLoggers;
use crate::finops::vantage::support::FakeVantageLogger;
use crate::finops::vantage_settings::is_vantage_setup;
use crate::finops::vantage_settings::is_vantage_setup_in_config;
use crate::finops::vantage_settings::is_vantage_setup_in_db;

use super::support::Database;
use super::support::callbacks;
use super::support::drop_table;
use super::support::seed_null;
use super::support::seed_value;

const STORED_SETTINGS: &str =
    r#"{"api_key":"sealed","integration_token":"sealed","base_url":"https://api.vantage.sh"}"#;

fn registered(list: VantageCallbackList) -> VantageLoggers {
    let loggers = VantageLoggers::new();
    let logger: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    loggers.register(logger, list);
    loggers
}

#[test]
fn config_setup_matches_only_the_vantage_callback_name() {
    let loggers = VantageLoggers::new();
    assert!(is_vantage_setup_in_config(
        &callbacks(&["vantage"]),
        &loggers
    ));
    assert!(is_vantage_setup_in_config(
        &callbacks(&["langfuse", "vantage"]),
        &loggers
    ));
    assert!(!is_vantage_setup_in_config(
        &callbacks(&["cloudzero"]),
        &loggers
    ));
    assert!(!is_vantage_setup_in_config(
        &callbacks(&["vantage_export"]),
        &loggers
    ));
    assert!(!is_vantage_setup_in_config(&[], &loggers));
}

#[test]
fn config_setup_matches_a_registered_vantage_logger() {
    assert!(!is_vantage_setup_in_config(&[], &VantageLoggers::new()));
    let loggers = registered(VantageCallbackList::Callbacks);
    assert!(is_vantage_setup_in_config(&[], &loggers));
    assert!(is_vantage_setup_in_config(
        &callbacks(&["vantage"]),
        &loggers
    ));
}

#[test]
fn config_setup_ignores_a_logger_outside_the_general_list() {
    let loggers = registered(VantageCallbackList::Success);
    assert!(!is_vantage_setup_in_config(&[], &loggers));
    assert!(is_vantage_setup_in_config(
        &callbacks(&["vantage"]),
        &loggers
    ));
}

#[tokio::test]
async fn db_setup_reports_a_stored_value() {
    let database = Database::provision().await;
    assert!(!is_vantage_setup_in_db(Some(database.client())).await);
    seed_value(&database, VANTAGE_SETTINGS_PARAM_NAME, STORED_SETTINGS).await;
    assert!(is_vantage_setup_in_db(Some(database.client())).await);
    database.close().await;
}

#[tokio::test]
async fn db_setup_reports_a_null_value_as_unconfigured() {
    let database = Database::provision().await;
    seed_null(&database, VANTAGE_SETTINGS_PARAM_NAME).await;
    assert!(!is_vantage_setup_in_db(Some(database.client())).await);
    database.close().await;
}

#[tokio::test]
async fn db_setup_reports_false_when_the_query_fails() {
    let database = Database::provision().await;
    drop_table(&database, "LiteLLM_Config").await;
    assert!(!is_vantage_setup_in_db(Some(database.client())).await);
    database.close().await;
}

#[tokio::test]
async fn db_setup_reports_false_without_a_database() {
    assert!(!is_vantage_setup_in_db(None).await);
}

#[tokio::test]
async fn setup_composes_the_config_and_database_sources() {
    let database = Database::provision().await;
    let loggers = VantageLoggers::new();
    assert!(
        !is_vantage_setup(
            Some(database.client()),
            &callbacks(&["cloudzero"]),
            &loggers
        )
        .await
    );
    assert!(is_vantage_setup(None, &callbacks(&["vantage"]), &loggers).await);
    assert!(is_vantage_setup(None, &[], &registered(VantageCallbackList::Callbacks)).await);
    seed_value(&database, VANTAGE_SETTINGS_PARAM_NAME, STORED_SETTINGS).await;
    assert!(is_vantage_setup(Some(database.client()), &[], &loggers).await);
    assert!(is_vantage_setup(Some(database.client()), &callbacks(&["langfuse"]), &loggers).await);
    database.close().await;
}
