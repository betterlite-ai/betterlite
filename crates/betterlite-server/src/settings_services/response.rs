use std::collections::BTreeMap;

use betterlite_config::sections::settings::{FieldSource, JsonValue};
use serde::Serialize;

use super::schema::FieldSchema;

// SettingsResponse and _SettingsWithSchema: the values a settings record renders and the schema the UI reads
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingsResponse {
    pub values: BTreeMap<String, JsonValue>,
    pub field_schema: FieldSchema,
}

// SSOSettingsResponse: the common response plus where each stored field's value came from
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SsoSettingsResponse {
    #[serde(flatten)]
    pub settings: SettingsResponse,
    pub provenance: BTreeMap<String, FieldSource>,
}

// _update_litellm_setting's return value: the caller's success message, the fixed status and the stored settings
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingsUpdateResponse {
    pub message: String,
    pub status: SettingsUpdateStatus,
    pub settings: BTreeMap<String, JsonValue>,
}

// the closed set of statuses the settings update routes return
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsUpdateStatus {
    Success,
}
