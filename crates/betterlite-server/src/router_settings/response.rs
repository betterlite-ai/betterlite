use std::collections::BTreeMap;

use betterlite_config::sections::settings::settings_resolution::source_for;
use betterlite_config::sections::settings::{FieldSource, JsonValue, SettingsStore};
use serde::Serialize;

use super::catalog::{
    FIELDS, PRIORITY_GROUP_STRATEGY, STRATEGIES, STRATEGY_DESCRIPTIONS, field_default,
    strategy_name,
};

#[derive(Debug, Clone, Serialize)]
pub struct RouterSettingsField {
    pub field_name: &'static str,
    pub field_type: &'static str,
    pub field_value: JsonValue,
    pub field_description: &'static str,
    pub field_default: JsonValue,
    pub options: Option<Vec<&'static str>>,
    pub ui_field_name: &'static str,
    pub link: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouterSettingsResponse {
    pub fields: Vec<RouterSettingsField>,
    pub current_values: BTreeMap<String, JsonValue>,
    pub routing_strategy_descriptions: BTreeMap<&'static str, &'static str>,
    pub routing_group_strategies: Vec<&'static str>,
    pub source: BTreeMap<String, FieldSource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouterFieldsResponse {
    pub fields: Vec<RouterSettingsField>,
    pub routing_strategy_descriptions: BTreeMap<&'static str, &'static str>,
    pub routing_group_strategies: Vec<&'static str>,
}

#[must_use]
pub fn fields_response() -> RouterFieldsResponse {
    RouterFieldsResponse {
        fields: catalog_fields(),
        routing_strategy_descriptions: STRATEGY_DESCRIPTIONS.into_iter().collect(),
        routing_group_strategies: group_strategies(),
    }
}

#[must_use]
pub fn settings_response(
    current_values: BTreeMap<String, JsonValue>,
    store: &SettingsStore,
) -> RouterSettingsResponse {
    let mut fields = catalog_fields();
    for field in &mut fields {
        if let Some(value) = current_values.get(field.field_name) {
            field.field_value.clone_from(value);
        }
    }
    // every key the router reports gets a source, including one the field catalog doesn't name
    let source = current_values
        .iter()
        .map(|(key, value)| {
            let default = field_default(key);
            (key.clone(), setting_source(store, key, value, &default))
        })
        .collect();
    RouterSettingsResponse {
        fields,
        current_values,
        routing_strategy_descriptions: STRATEGY_DESCRIPTIONS.into_iter().collect(),
        routing_group_strategies: group_strategies(),
        source,
    }
}

// _router_setting_source: a resolved store decides; otherwise a value the router holds is its default
fn setting_source(
    store: &SettingsStore,
    key: &str,
    current_value: &JsonValue,
    field_default: &JsonValue,
) -> FieldSource {
    let source = source_for(store, key, field_default);
    if source != FieldSource::Unset {
        return source;
    }
    if *current_value == JsonValue::Null {
        FieldSource::Unset
    } else {
        FieldSource::Default
    }
}

fn catalog_fields() -> Vec<RouterSettingsField> {
    FIELDS
        .iter()
        .map(|definition| RouterSettingsField {
            field_name: definition.name,
            field_type: definition.kind,
            field_value: JsonValue::Null,
            field_description: definition.description,
            field_default: definition.default.json(),
            options: (definition.name == "routing_strategy").then(strategy_options),
            ui_field_name: definition.ui_name,
            link: definition.link,
        })
        .collect()
}

fn strategy_options() -> Vec<&'static str> {
    STRATEGIES
        .iter()
        .copied()
        .map(strategy_name)
        .collect::<Vec<_>>()
}

fn group_strategies() -> Vec<&'static str> {
    let mut strategies = strategy_options();
    strategies.push(PRIORITY_GROUP_STRATEGY);
    strategies
}
