use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};

use super::error::SettingsSchemaError;

// JsonSchemaNode: the pydantic-v2 schema node LiteLLM reads, with every key it never reads kept in passthrough
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct SchemaNode {
    #[serde(rename = "type")]
    pub schema_type: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<JsonValue>>,
    #[serde(rename = "anyOf")]
    pub any_of: Option<Vec<SchemaNode>>,
    pub items: Option<SchemaItems>,
    pub properties: Option<BTreeMap<String, SchemaNode>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, JsonValue>,
}

// JsonSchemaItems: the items mapping a direct array field keeps whole, or only the enum a $ref definition contributes
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct SchemaItems {
    #[serde(rename = "$ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<JsonValue>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, JsonValue>,
}

// JsonSchemaPropertyEntry: what the UI reads for one field, the outer description beside the resolved variant's type
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SchemaPropertyEntry {
    pub description: String,
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<SchemaItems>,
}

// _RootSchema: a record type's pinned schema, whose two definition maps differ because pydantic v2 emits them differently
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RootSchema {
    pub description: String,
    pub properties: BTreeMap<String, SchemaNode>,
    pub nested_defs: BTreeMap<String, SchemaNode>,
    pub defs: BTreeMap<String, SchemaNode>,
}

// nested_defs_out's entry for one legacy definition
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NestedDefinition {
    pub description: String,
    pub properties: BTreeMap<String, NestedProperty>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NestedProperty {
    pub description: String,
}

// field_schema: the root description and projected properties, with the legacy nested sections spread beside them
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldSchema {
    pub description: String,
    pub properties: BTreeMap<String, SchemaPropertyEntry>,
    #[serde(flatten)]
    pub nested_definitions: BTreeMap<String, NestedDefinition>,
}

// _resolve_non_null_variant: pydantic renders Optional fields under anyOf, and the first non-null variant describes the field
#[must_use]
pub fn resolve_non_null_variant(field_info: &SchemaNode) -> &SchemaNode {
    let Some(variants) = field_info.any_of.as_ref() else {
        return field_info;
    };
    variants
        .iter()
        .find(|variant| variant.schema_type.as_deref() != Some("null"))
        .unwrap_or(field_info)
}

// _schema_items_entry: a $ref resolves by its final path segment and contributes only its definition's enum
#[must_use]
pub fn schema_items_entry(
    resolved: &SchemaNode,
    defs: &BTreeMap<String, SchemaNode>,
) -> Option<SchemaItems> {
    let items = resolved.items.clone()?;
    let Some(reference) = items.reference.clone() else {
        return Some(items);
    };
    let name = reference.rsplit('/').next();
    let definition = defs.get(name?)?;
    let enum_values = definition.enum_values.clone()?;
    Some(SchemaItems {
        reference: None,
        enum_values: Some(enum_values),
        passthrough: BTreeMap::new(),
    })
}

#[must_use]
pub fn schema_property_entry(
    field_info: &SchemaNode,
    defs: &BTreeMap<String, SchemaNode>,
) -> SchemaPropertyEntry {
    let resolved = resolve_non_null_variant(field_info);
    SchemaPropertyEntry {
        description: field_info.description.clone().unwrap_or_default(),
        schema_type: resolved
            .schema_type
            .as_deref()
            .unwrap_or("string")
            .to_owned(),
        items: schema_items_entry(resolved, defs),
    }
}

// _root_schema: a $defs entry exists in the raw schema exactly when pydantic v2 emitted modern definitions, and it then owns lookups
pub fn root_schema(raw: &JsonValue) -> Result<RootSchema, SettingsSchemaError> {
    let node: SchemaNode = serde_json::from_value(raw.clone())
        .map_err(|source| SettingsSchemaError::InvalidSchema { source })?;
    let properties = node
        .properties
        .ok_or(SettingsSchemaError::MissingProperties)?;
    let nested_defs = definition_map(&node.passthrough, "definitions")?;
    let defs = if node.passthrough.contains_key("$defs") {
        definition_map(&node.passthrough, "$defs")?
    } else {
        nested_defs.clone()
    };
    Ok(RootSchema {
        description: node.description.unwrap_or_default(),
        properties,
        nested_defs,
        defs,
    })
}

fn definition_map(
    passthrough: &BTreeMap<String, JsonValue>,
    section: &'static str,
) -> Result<BTreeMap<String, SchemaNode>, SettingsSchemaError> {
    match passthrough.get(section) {
        None | Some(JsonValue::Null) => Ok(BTreeMap::new()),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|source| SettingsSchemaError::InvalidDefinitions { section, source }),
    }
}
