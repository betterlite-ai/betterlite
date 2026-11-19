use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::key_fields::values::LiteLLMKeyType;

// an absent key type leaves no column behind, and only the three presets add their route group
#[must_use]
pub fn handle_key_type(
    key_type: Option<LiteLLMKeyType>,
    mut columns: BTreeMap<String, JsonData>,
) -> BTreeMap<String, JsonData> {
    let Some(key_type) = key_type else {
        columns.remove("key_type");
        return columns;
    };
    columns.insert(
        "key_type".to_owned(),
        JsonData::String(key_type_text(key_type).to_owned()),
    );
    let preset = preset_routes(key_type);
    if let Some(route) = preset {
        columns.insert(
            "allowed_routes".to_owned(),
            JsonData::Array(vec![JsonData::String(route.to_owned())]),
        );
    }
    columns
}

const fn key_type_text(key_type: LiteLLMKeyType) -> &'static str {
    match key_type {
        LiteLLMKeyType::LlmApi => "llm_api",
        LiteLLMKeyType::Management => "management",
        LiteLLMKeyType::ReadOnly => "read_only",
        LiteLLMKeyType::Default => "default",
    }
}

const fn preset_routes(key_type: LiteLLMKeyType) -> Option<&'static str> {
    match key_type {
        LiteLLMKeyType::LlmApi => Some("llm_api_routes"),
        LiteLLMKeyType::Management => Some("management_routes"),
        LiteLLMKeyType::ReadOnly => Some("info_routes"),
        LiteLLMKeyType::Default => None,
    }
}
