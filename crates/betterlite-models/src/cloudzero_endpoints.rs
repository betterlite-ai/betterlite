use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::decisions::Json;
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CloudZeroInitRequest {
    pub api_key: String,
    pub connection_id: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CloudZeroInitResponse {
    pub message: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CloudZeroExportRequest {
    pub limit: Option<i64>,
    #[serde(default = "default_operation")]
    pub operation: String,
    pub start_time_utc: Option<DateTime<Utc>>,
    pub end_time_utc: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CloudZeroExportResponse {
    pub message: String,
    pub status: String,
    pub records_exported: Option<i64>,
    pub dry_run_data: Option<Json>,
    pub summary: Option<Json>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CloudZeroSettingsView {
    pub api_key_masked: Option<String>,
    pub connection_id: Option<String>,
    pub timezone: Option<String>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CloudZeroSettingsUpdate {
    pub api_key: Option<String>,
    pub connection_id: Option<String>,
    pub timezone: Option<String>,
}

fn default_timezone() -> String {
    let mut timezone = String::new();
    timezone.push_str("UTC");
    timezone
}

fn default_operation() -> String {
    let mut operation = String::new();
    operation.push_str("replace_hourly");
    operation
}
