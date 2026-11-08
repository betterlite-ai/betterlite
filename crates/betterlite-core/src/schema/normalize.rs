use super::bounds::{SchemaCheck, check_schema};
use super::{SchemaError, SchemaJson, SchemaLimits};

const PARAMETERS: &str = "parameters";
const FUNCTION: &str = "function";

// provider schemas spell JSON Schema types in upper case; LiteLLM lowercases them before it sends them on
pub fn normalize_json_schema_types(
    schema: &SchemaJson,
    limits: &SchemaLimits,
) -> Result<SchemaJson, SchemaError> {
    check_schema(schema, limits, SchemaCheck::Size)?;
    Ok(normalize(schema, 0, limits.max_depth))
}

pub fn normalize_tool_schema(
    tool: &SchemaJson,
    limits: &SchemaLimits,
) -> Result<SchemaJson, SchemaError> {
    check_schema(tool, limits, SchemaCheck::Size)?;
    let SchemaJson::Object(container) = tool else {
        return Ok(tool.clone());
    };
    let mut normalized = container.clone();
    let Some(SchemaJson::Object(function)) = container.get(FUNCTION) else {
        return Ok(SchemaJson::Object(normalized));
    };
    let mut rewritten = function.clone();
    if let Some(parameters) = rewritten.get(PARAMETERS) {
        let mapped = normalize(parameters, 0, limits.max_depth);
        rewritten.insert(String::from(PARAMETERS), mapped);
    }
    normalized.insert(String::from(FUNCTION), SchemaJson::Object(rewritten));
    Ok(SchemaJson::Object(normalized))
}

fn normalize(value: &SchemaJson, depth: usize, max_depth: usize) -> SchemaJson {
    if depth >= max_depth {
        return value.clone();
    }
    match value {
        SchemaJson::Array(items) => SchemaJson::Array(
            items
                .iter()
                .map(|item| normalize(item, depth.saturating_add(1), max_depth))
                .collect(),
        ),
        SchemaJson::Object(object) => normalize_object(object, depth, max_depth),
        SchemaJson::Null | SchemaJson::Bool(_) | SchemaJson::Number(_) | SchemaJson::String(_) => {
            value.clone()
        }
    }
}

fn normalize_object(
    object: &serde_json::Map<String, SchemaJson>,
    depth: usize,
    max_depth: usize,
) -> SchemaJson {
    let mut normalized = object.clone();
    for (key, child) in object {
        let mapped = normalize_typed_entry(key, child, depth, max_depth)
            .unwrap_or_else(|| normalize(child, depth.saturating_add(1), max_depth));
        drop(normalized.insert(key.clone(), mapped));
    }
    SchemaJson::Object(normalized)
}

fn normalize_typed_entry(
    key: &str,
    value: &SchemaJson,
    depth: usize,
    max_depth: usize,
) -> Option<SchemaJson> {
    match (key, value) {
        ("type", SchemaJson::String(name)) => {
            normalize_type(name).map(|mapped| SchemaJson::String(mapped.to_owned()))
        }
        // a property value sits one level under its schema, not two, as LiteLLM recurses into it directly
        ("properties", SchemaJson::Object(properties)) => {
            Some(normalize_object(properties, depth, max_depth))
        }
        _ => None,
    }
}

fn normalize_type(name: &str) -> Option<&'static str> {
    match name {
        "BOOLEAN" => Some("boolean"),
        "STRING" => Some("string"),
        "ARRAY" => Some("array"),
        "OBJECT" => Some("object"),
        "NUMBER" => Some("number"),
        "INTEGER" => Some("integer"),
        "NULL" => Some("null"),
        _ => None,
    }
}
