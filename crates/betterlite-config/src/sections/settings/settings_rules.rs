use std::collections::BTreeMap;

use super::settings_resolution::JsonValue as Value;

use super::descriptors::FieldSource;
use super::settings_store::SettingsStore;

#[must_use]
pub fn coerce_bool(value: Value) -> Value {
    match value {
        Value::Null | Value::Boolean(_) => value,
        Value::Text(text) => Value::Boolean(text.eq_ignore_ascii_case("true")),
        Value::SignedInteger(_) | Value::UnsignedInteger(_) | Value::Decimal(_) | Value::Array(_) | Value::Object(_) => Value::Boolean(!value.is_null()),
    }
}

#[must_use]
pub fn source_for(store: &SettingsStore, key: &str, default: Option<&Value>) -> FieldSource {
    match store.source(key) {
        FieldSource::Unset if default.is_some() => FieldSource::Default,
        FieldSource::Config => FieldSource::Config,
        FieldSource::Db => FieldSource::Db,
        FieldSource::Env => FieldSource::Env,
        FieldSource::Default => FieldSource::Default,
        FieldSource::Unset => FieldSource::Unset,
    }
}

pub(crate) const UI_SETTINGS_FIELDS: [&str; 10] = [
    "allow_public_health_readiness_details",
    "forward_client_headers_to_llm_api",
    "forward_llm_provider_auth_headers",
    "disable_agents_for_internal_users",
    "allow_agents_for_team_admins",
    "disable_vector_stores_for_internal_users",
    "allow_vector_stores_for_team_admins",
    "disable_custom_api_keys",
    "disable_key_generate_for_org_admin",
    "team_admin_editable_team_fields",
];

pub(crate) fn db_row_for<'section>(section: &'section str, key: &str) -> &'section str {
    if section == "general_settings" && UI_SETTINGS_FIELDS.contains(&key) {
        "ui_settings"
    } else {
        section
    }
}

pub(crate) const fn empty_values() -> BTreeMap<String, Value> {
    BTreeMap::new()
}
