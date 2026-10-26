use std::collections::BTreeMap;

pub mod alerting;
pub mod coerce_bool;
pub mod descriptors;
pub mod settings_resolution;
pub mod settings_rules;
pub mod settings_store;

pub type FieldDescriptor = descriptors::FieldDescriptor;
pub type FieldSource = descriptors::FieldSource;
pub type JsonValue = settings_resolution::JsonValue;
pub type Resolved = settings_resolution::Resolved;
pub type SettingValue = settings_resolution::SettingValue;
pub type ConfigOwnedKeyError = settings_store::ConfigOwnedKeyError;
pub type SettingsStore = settings_store::SettingsStore;
pub type Section = settings_store::Section;

#[must_use]
pub fn resolve_fields(
    descriptors: &[FieldDescriptor],
    database: &BTreeMap<String, JsonValue>,
    environment: &BTreeMap<String, String>,
    empty_db_is_set: bool,
) -> (BTreeMap<String, Option<String>>, BTreeMap<String, FieldSource>) {
    descriptors::resolve_fields(descriptors, database, environment, empty_db_is_set)
}

#[must_use]
pub fn config_ownership_message(section: &str, key: &str, shadows_db_value: bool) -> String {
    settings_store::config_ownership_message(section, key, shadows_db_value)
}

#[must_use]
pub fn source_for(store: &SettingsStore, key: &str, default: Option<&JsonValue>) -> FieldSource {
    settings_rules::source_for(store, key, default)
}

#[must_use]
pub fn coerce_bool(value: JsonValue) -> JsonValue {
    coerce_bool::coerce_bool(value)
}

crate::sections::stub_section!();
