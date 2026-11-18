#![cfg(test)]

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use crate::roi_settings::repository::SettingsRepository as _;
use crate::roi_settings::store::SETTINGS_KEY;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::catalog;
use crate::roi_settings::support::disconnected_repository;
use crate::roi_settings::support::get_request;
use crate::roi_settings::support::settings_app;

#[tokio::test]
async fn config_repository_without_a_database_is_db_not_connected() {
    let app = settings_app(None, Some(catalog(Vec::new())));
    let reply = call(app, get_request(Some(&admin_caller()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[tokio::test]
async fn config_repository_reads_and_writes_through_the_config_table() {
    let repository = disconnected_repository();
    repository.param(SETTINGS_KEY).await.unwrap_err();
    repository
        .set_param(SETTINGS_KEY, JsonData::Null)
        .await
        .unwrap_err();
}
