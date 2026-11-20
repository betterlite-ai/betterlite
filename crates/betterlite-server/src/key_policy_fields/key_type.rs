use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::key_fields::values::LiteLLMKeyType;

// an absent key type leaves no column behind, and only the three presets add their route group
#[must_use]
pub fn handle_key_type(
    key_type: Option<LiteLLMKeyType>,
    mut columns: BTreeMap<String, JsonData>,
) -> BTreeMap<String, JsonData> {
    let Some(requested) = key_type else {
        columns.remove("key_type");
        return columns;
    };
    let (text, preset) = key_type_parts(requested);
    columns.insert("key_type".to_owned(), JsonData::String(text.to_owned()));
    if let Some(route) = preset {
        columns.insert(
            "allowed_routes".to_owned(),
            JsonData::Array(vec![JsonData::String(route.to_owned())]),
        );
    }
    columns
}

const fn key_type_parts(key_type: LiteLLMKeyType) -> (&'static str, Option<&'static str>) {
    match key_type {
        LiteLLMKeyType::LlmApi => ("llm_api", Some("llm_api_routes")),
        LiteLLMKeyType::Management => ("management", Some("management_routes")),
        LiteLLMKeyType::ReadOnly => ("read_only", Some("info_routes")),
        LiteLLMKeyType::Default => ("default", None),
    }
}
