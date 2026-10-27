use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Serialize;

use super::settings_rules::JsonValue;
use super::settings_store::SettingsStore;

pub mod alerting;

mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldSource {
    Config,
    Db,
    Env,
    Default,
    Unset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDescriptor {
    pub field_name: &'static str,
    pub db_key: &'static str,
    pub env_var: &'static str,
    pub is_secret: bool,
    pub default: Option<&'static str>,
}

// alerting clears a webhook by storing "" and leaves the env var set; SSO unsets the env var as well
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyDbValue {
    Cleared,
    Absent,
}

#[derive(Clone, Debug)]
pub struct ResolvedField {
    pub field_name: &'static str,
    pub value: Option<SecretString>,
    pub source: FieldSource,
}

#[must_use]
pub fn resolve_fields(
    descriptors: &[FieldDescriptor],
    stored: &BTreeMap<String, JsonValue>,
    environment: &BTreeMap<String, SecretString>,
    empty: EmptyDbValue,
) -> Vec<ResolvedField> {
    let mut fields = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        fields.push(resolve_field(descriptor, stored, environment, empty));
    }
    fields
}

fn resolve_field(
    descriptor: &FieldDescriptor,
    stored: &BTreeMap<String, JsonValue>,
    environment: &BTreeMap<String, SecretString>,
    empty: EmptyDbValue,
) -> ResolvedField {
    // Python's str.strip also drops the four ASCII separators U+001C to U+001F
    let is_blank = |text: &str| {
        text.trim_matches(|character: char| {
            character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
        })
        .is_empty()
    };
    let stored_text = match (stored.get(descriptor.db_key), empty) {
        (Some(JsonValue::String(text)), EmptyDbValue::Absent) if !is_blank(text) => {
            Some(text.clone())
        }
        (None | Some(JsonValue::Null), _) | (Some(_), EmptyDbValue::Absent) => None,
        (Some(value), EmptyDbValue::Cleared) => Some(python_text(value)),
    };
    let environment_text = environment
        .get(descriptor.env_var)
        .map(secrecy::ExposeSecret::expose_secret)
        .filter(|text| !is_blank(text));
    let resolved = stored_text
        .map(|text| (text, FieldSource::Db))
        .or_else(|| environment_text.map(|text| (text.to_owned(), FieldSource::Env)))
        .or_else(|| {
            descriptor
                .default
                .map(|default| (default.to_owned(), FieldSource::Default))
        });
    let (value, source) = resolved.map_or((None, FieldSource::Unset), |(value, source)| {
        (Some(SecretString::from(value)), source)
    });
    ResolvedField {
        field_name: descriptor.field_name,
        value,
        source,
    }
}

// LiteLLM hands the stored value to str(), so booleans read True and False
fn python_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Bool(true) => String::from("True"),
        JsonValue::Bool(false) => String::from("False"),
        JsonValue::Null => String::from("None"),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
    }
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
