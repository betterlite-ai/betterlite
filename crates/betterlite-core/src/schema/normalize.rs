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
        SchemaJson::Object(object) => normalize_map(object, |key, child| {
            if key == "type"
                && let SchemaJson::String(name) = child
                && let Some(lower) = normalize_type(name)
            {
                return SchemaJson::String(lower.to_owned());
            }
            // every property value is a schema on its own, and the property names are never read as keywords
            if key == "properties"
                && let SchemaJson::Object(properties) = child
            {
                return normalize_map(properties, |_name, property| {
                    normalize(property, depth.saturating_add(1), max_depth)
                });
            }
            normalize(child, depth.saturating_add(1), max_depth)
        }),
        SchemaJson::Null | SchemaJson::Bool(_) | SchemaJson::Number(_) | SchemaJson::String(_) => {
            value.clone()
        }
    }
}

fn normalize_map<F>(object: &serde_json::Map<String, SchemaJson>, map_child: F) -> SchemaJson
where
    F: Fn(&str, &SchemaJson) -> SchemaJson,
{
    let mut normalized = serde_json::Map::with_capacity(object.len());
    for (key, child) in object {
        drop(normalized.insert(key.clone(), map_child(key, child)));
    }
    SchemaJson::Object(normalized)
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
