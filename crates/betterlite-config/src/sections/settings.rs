use std::collections::BTreeMap;
use std::num::FpCategory;
use std::slice::from_ref;

use secrecy::SecretString;

pub mod changed_section_keys;
pub mod coerce_bool;
pub mod settings_resolution;
pub mod settings_rules;
pub mod settings_store;

pub type FieldDescriptor = settings_resolution::FieldDescriptor;
pub type FieldSource = settings_resolution::FieldSource;
pub type DbRow = settings_rules::DbRow;
pub type JsonValue = settings_rules::JsonValue;
pub type Section = settings_rules::Section;
pub type ConfigOwnedKeyError = settings_store::ConfigOwnedKeyError;
pub type SettingsStore = settings_store::SettingsStore;

#[must_use]
pub fn resolve_fields(
    descriptors: &[FieldDescriptor],
    stored: &BTreeMap<String, JsonValue>,
    environment: &BTreeMap<String, SecretString>,
    empty: settings_resolution::EmptyDbValue,
) -> Vec<settings_resolution::ResolvedField> {
    let mut fields = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        let single =
            settings_resolution::resolve_fields(from_ref(descriptor), stored, environment, empty);
        fields.extend(single);
    }
    fields
}

#[must_use]
pub fn config_ownership_message(section: Section, key: &str, shadows_db_value: bool) -> String {
    let stored = if shadows_db_value {
        " The value stored in the database for it is ignored and will never be applied."
    } else {
        ""
    };
    format!(
        "{}.{key} is set in the config file, so the config file owns it and it cannot be changed here.{stored} Edit the config file to change it, or remove it from the file to let the database own it.",
        section.name()
    )
}

#[must_use]
pub fn source_for(store: &SettingsStore, key: &str, default: &JsonValue) -> FieldSource {
    let source = store.resolution(key).source();
    if source == FieldSource::Unset && *default != JsonValue::Null {
        FieldSource::Default
    } else {
        source
    }
}

#[must_use]
pub fn coerce_bool(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Null | JsonValue::Bool(_) => value,
        JsonValue::String(text) => JsonValue::Bool(text.to_lowercase() == "true"),
        JsonValue::Number(number) => JsonValue::Bool(
            number
                .as_f64()
                .is_some_and(|float| float.classify() != FpCategory::Zero),
        ),
        JsonValue::Array(items) => JsonValue::Bool(!items.is_empty()),
        JsonValue::Object(entries) => JsonValue::Bool(!entries.is_empty()),
    }
}

crate::sections::stub_section!();
