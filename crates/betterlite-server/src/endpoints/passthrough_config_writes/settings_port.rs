use std::future::Future;

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;
use thiserror::Error;

// LiteLLM's ConfigFieldInfo: one general_settings field as the store currently holds it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFieldInfo {
    pub field_name: String,
    pub field_value: JsonData,
}

// LiteLLM's ConfigFieldUpdate: the whole field replaced by the list the route built
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFieldUpdate {
    pub field_name: String,
    pub field_value: JsonData,
}

// the store's refusal keeps the status and detail it chose, so the route answers what a LiteLLM HTTPException would
#[derive(Debug, Clone, Error)]
#[error("the general settings store refused the call with status {status}")]
pub struct GeneralSettingsError {
    pub status: StatusCode,
    pub detail: String,
}

// get_config_general_settings and update_config_general_settings live in proxy_server.py; the routes reach them here
pub trait GeneralSettingsStore: Send + Sync {
    fn get_field(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<ConfigFieldInfo, GeneralSettingsError>> + Send;

    fn update_field(
        &self,
        update: ConfigFieldUpdate,
    ) -> impl Future<Output = Result<(), GeneralSettingsError>> + Send;
}
