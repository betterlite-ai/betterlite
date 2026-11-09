use std::collections::BTreeMap;

use betterlite_openai::chat::Tool;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::oci_chat_wire::tool_schema::resolved_tool_schema;
use crate::oci_chat_wire::types::cohere::{CohereParameterDefinition, CohereTool};
use crate::oci_common::OciError;
use crate::oci_common::schema::{OCI_JSON_TO_PYTHON_TYPES, enrich_cohere_description};

pub fn adapt_cohere_tools(tools: &[Tool]) -> Result<Vec<CohereTool>, OciError> {
    let mut adapted = Vec::with_capacity(tools.len());
    for tool in tools {
        let definition = tool.function.as_ref();
        let resolved = resolved_tool_schema(definition)?;
        adapted.push(CohereTool {
            name: definition.map_or_else(String::new, |function| function.name.clone()),
            description: definition
                .and_then(|function| function.description.clone())
                .unwrap_or_default(),
            parameter_definitions: parameter_definitions(&resolved),
        });
    }
    Ok(adapted)
}

fn parameter_definitions(resolved: &Json) -> BTreeMap<String, CohereParameterDefinition> {
    let Some(object) = resolved.as_object() else {
        return BTreeMap::new();
    };
    let Some(properties) = object.get("properties").and_then(Json::as_object) else {
        return BTreeMap::new();
    };
    let required: &[Json] = match object.get("required") {
        Some(Json::List(names)) => names,
        _ => &[],
    };
    properties
        .iter()
        .map(|(property_name, property_schema)| {
            (
                property_name.clone(),
                parameter_definition(property_name, property_schema, required),
            )
        })
        .collect()
}

fn parameter_definition(
    name: &str,
    property_schema: &Json,
    required: &[Json],
) -> CohereParameterDefinition {
    let fields = property_schema.as_object();
    let description = fields
        .and_then(|schema_fields| schema_fields.get("description"))
        .and_then(Json::as_str)
        .unwrap_or_default();
    let kind = fields
        .and_then(|schema_fields| schema_fields.get("type"))
        .and_then(Json::as_str)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("string");
    let mapped = OCI_JSON_TO_PYTHON_TYPES
        .iter()
        .find(|(json_type, _)| *json_type == kind)
        .map_or(kind, |(_, python_type)| *python_type);
    let empty = Object::new();
    CohereParameterDefinition {
        description: enrich_cohere_description(description, fields.unwrap_or(&empty)),
        kind: mapped.to_owned(),
        is_required: required
            .iter()
            .any(|entry| matches!(entry, Json::Text(text) if text == name)),
    }
}
