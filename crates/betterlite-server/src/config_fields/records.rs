use betterlite_config::sections::settings::{FieldSource, JsonValue};
use serde::{Deserialize, Serialize};

pub const GENERAL_SETTINGS: &str = "general_settings";

// the Literal["general_settings"] both request records declare
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigType {
    GeneralSettings,
}

// ConfigFieldUpdate: field_name, field_value, config_type, in that declaration order
#[derive(Clone, Debug, Deserialize)]
pub struct ConfigFieldUpdate {
    pub field_name: String,
    pub field_value: JsonValue,
    pub config_type: ConfigType,
}

// ConfigFieldDelete: config_type, field_name
#[derive(Clone, Debug, Deserialize)]
pub struct ConfigFieldDelete {
    pub config_type: ConfigType,
    pub field_name: String,
}

// the upserted LiteLLM_Config row the update and delete routes answer with
#[derive(Clone, Debug, Serialize)]
pub struct GeneralSettingsRow {
    pub param_name: &'static str,
    pub param_value: JsonValue,
}

// the dict the two UI litellm helpers answer instead of a config row
#[derive(Clone, Debug, Serialize)]
pub struct UiFieldResponse {
    pub message: String,
    pub status: &'static str,
}

// a UI litellm field changes litellm itself, so update and delete answer two different bodies
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ConfigFieldUpdateResponse {
    Row(GeneralSettingsRow),
    Ui(UiFieldResponse),
}

#[derive(Debug, Serialize)]
pub struct ConfigFieldInfo {
    pub field_name: String,
    pub field_value: JsonValue,
    pub source: FieldSource,
    pub editable: bool,
}

// ConfigList, in the pydantic field order; every field serializes even when it is null
#[derive(Debug, Serialize)]
pub struct ConfigList {
    pub field_name: String,
    pub field_type: &'static str,
    pub field_description: &'static str,
    pub field_value: JsonValue,
    pub stored_in_db: Option<bool>,
    pub field_default_value: JsonValue,
    pub premium_field: bool,
    pub nested_fields: Option<Vec<FieldDetail>>,
    pub field_options: Option<Vec<&'static str>>,
    pub field_tab: Option<&'static str>,
    pub source: FieldSource,
    pub editable: bool,
}

// FieldDetail: one PassThroughGenericEndpoint annotation
#[derive(Debug, Serialize)]
pub struct FieldDetail {
    pub field_name: &'static str,
    pub field_type: &'static str,
    pub field_description: &'static str,
    pub field_default_value: JsonValue,
    pub stored_in_db: Option<bool>,
}
