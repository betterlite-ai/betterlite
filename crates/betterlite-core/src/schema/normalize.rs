use super::bounds::check_size;
use super::{SchemaError, SchemaJson, SchemaLimits};

const PARAMETERS: &str = "parameters";
const FUNCTION: &str = "function";

// provider schemas spell JSON Schema types in upper case; LiteLLM lowercases them before it sends them on
pub fn normalize_json_schema_types(
    schema: &SchemaJson,
    limits: &SchemaLimits,
) -> Result<SchemaJson, SchemaError> {
    check_size(schema, limits)?;
    Ok(normalize(schema, 0, limits.max_depth))
}

pub fn normalize_tool_schema(
    tool: &SchemaJson,
    limits: &SchemaLimits,
) -> Result<SchemaJson, SchemaError> {
    check_size(tool, limits)?;
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
        SchemaJson::Object(object) => SchemaJson::Object(
            object
                .iter()
                .map(|(key, child)| (key.clone(), normalize_entry(key, child, depth, max_depth)))
                .collect(),
        ),
        SchemaJson::Null | SchemaJson::Bool(_) | SchemaJson::Number(_) | SchemaJson::String(_) => {
            value.clone()
        }
    }
}

fn normalize_entry(key: &str, value: &SchemaJson, depth: usize, max_depth: usize) -> SchemaJson {
    match (key, value) {
        ("type", SchemaJson::String(name)) => normalize_type(name).map_or_else(
            || value.clone(),
            |mapped| SchemaJson::String(mapped.to_owned()),
        ),
        // a property value sits one level under its schema, not two, as LiteLLM recurses into it directly
        ("properties", SchemaJson::Object(properties)) => SchemaJson::Object(
            properties
                .iter()
                .map(|(name, property)| {
                    (
                        name.clone(),
                        normalize(property, depth.saturating_add(1), max_depth),
                    )
                })
                .collect(),
        ),
        _ => normalize(value, depth.saturating_add(1), max_depth),
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
