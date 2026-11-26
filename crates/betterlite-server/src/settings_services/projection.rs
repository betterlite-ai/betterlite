use std::collections::BTreeMap;

use super::records::SettingsRecord;
use super::response::SettingsResponse;
use super::schema::{
    FieldSchema, NestedDefinition, NestedProperty, RootSchema, SchemaNode, schema_property_entry,
};

// _get_settings_with_schema's response: the record's dumped values and the pinned schema's descriptions for the UI
#[must_use]
pub fn settings_with_schema<R>(record: &R) -> SettingsResponse
where
    R: SettingsRecord + ?Sized,
{
    let root = record.schema();
    SettingsResponse {
        values: record.values(),
        field_schema: field_schema(root),
    }
}

fn field_schema(root: &RootSchema) -> FieldSchema {
    let properties: BTreeMap<_, _> = root
        .properties
        .iter()
        .map(|(field_name, field_info)| {
            let entry = schema_property_entry(field_info, &root.defs);
            (field_name.clone(), entry)
        })
        .collect();
    let nested_definitions: BTreeMap<_, _> = root
        .nested_defs
        .iter()
        .map(|(def_name, def_schema)| (def_name.clone(), nested_definition(def_schema)))
        .collect();
    FieldSchema {
        description: root.description.clone(),
        properties,
        nested_definitions,
    }
}

// nested_defs_out: a legacy definition contributes only its own description and its properties' descriptions
fn nested_definition(definition: &SchemaNode) -> NestedDefinition {
    let properties: BTreeMap<_, _> = definition
        .properties
        .iter()
        .flatten()
        .map(|(prop_name, prop_info)| {
            let nested = NestedProperty {
                description: prop_info.description.clone().unwrap_or_default(),
            };
            (prop_name.clone(), nested)
        })
        .collect();
    NestedDefinition {
        description: definition.description.clone().unwrap_or_default(),
        properties,
    }
}
