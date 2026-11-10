use crate::arguments::json::{Json, Object};
use crate::bedrock_tools::capabilities::BedrockToolCapabilities;
use crate::bedrock_tools::error::BedrockToolError;
use crate::definitions::expand::unpack_defs;
use crate::definitions::legacy::DEFS_MAX_INLINED_BYTES;

const VALID_ROOT_TYPES: [&str; 7] = [
    "array", "boolean", "integer", "null", "number", "object", "string",
];
const CUSTOM_TYPE_NESTING: usize = 1024;

// Bedrock keeps only these four schema keys, so everything else a client sent is dropped before the request.
pub fn parameter_schema(
    parameters: &Json,
    capabilities: BedrockToolCapabilities,
) -> Result<Json, BedrockToolError> {
    let Json::Object(object) = parameters else {
        return Err(BedrockToolError::WrongType {
            field: "parameters",
        });
    };
    let mut fields = object.clone();
    let defs = match fields.shift_remove("$defs") {
        None | Some(Json::Null) => Object::new(),
        Some(Json::Object(defs)) => defs,
        Some(_) => {
            return Err(BedrockToolError::WrongType { field: "$defs" });
        }
    };
    let mut expanded = unpack_defs(fields, &defs, Some(DEFS_MAX_INLINED_BYTES))
        .map_err(|source| BedrockToolError::Schema { source })?;
    normalize_custom_types(&mut expanded, 0);
    let mut schema = Object::new();
    schema.insert("type".to_owned(), root_type(&expanded)?);
    schema.insert("properties".to_owned(), properties(&expanded)?);
    schema.insert("required".to_owned(), required(&expanded)?);
    if capabilities.supports_strict_tools
        && let Some(additional) = expanded
            .get("additionalProperties")
            .filter(|value| !matches!(value, Json::Null))
    {
        schema.insert("additionalProperties".to_owned(), additional.clone());
    }
    Ok(Json::Object(schema))
}

fn root_type(parameters: &Object) -> Result<Json, BedrockToolError> {
    match parameters.get("type") {
        Some(Json::Text(name)) if VALID_ROOT_TYPES.contains(&name.as_str()) => {
            Ok(Json::Text(name.clone()))
        }
        Some(Json::Text(_) | Json::Null) | None => Ok(Json::Text("object".to_owned())),
        Some(_) => Err(BedrockToolError::WrongType { field: "type" }),
    }
}

fn properties(parameters: &Object) -> Result<Json, BedrockToolError> {
    match parameters.get("properties") {
        None | Some(Json::Null) => Ok(Json::Object(Object::new())),
        Some(Json::Object(fields)) => Ok(Json::Object(fields.clone())),
        Some(_) => Err(BedrockToolError::WrongType {
            field: "properties",
        }),
    }
}

fn required(parameters: &Object) -> Result<Json, BedrockToolError> {
    match parameters.get("required") {
        None | Some(Json::Null) => Ok(Json::List(Vec::new())),
        Some(Json::List(items)) if items.iter().all(|item| item.as_str().is_some()) => {
            Ok(Json::List(items.clone()))
        }
        Some(_) => Err(BedrockToolError::WrongType { field: "required" }),
    }
}

// Claude Code sends type custom for its own tools; Bedrock accepts only standard JSON Schema types.
fn normalize_custom_types(schema: &mut Object, depth: usize) {
    if depth >= CUSTOM_TYPE_NESTING {
        return;
    }
    if matches!(schema.get("type"), Some(Json::Text(name)) if name == "custom") {
        schema.insert("type".to_owned(), Json::Text("object".to_owned()));
    }
    let next = depth.saturating_add(1);
    for key in ["items", "additionalProperties"] {
        if let Some(child) = schema.get_mut(key) {
            normalize_node(child, next);
        }
    }
    if let Some(Json::Object(fields)) = schema.get_mut("properties") {
        for child in fields.values_mut() {
            normalize_node(child, next);
        }
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(Json::List(items)) = schema.get_mut(key) {
            for child in items {
                normalize_node(child, next);
            }
        }
    }
}

fn normalize_node(node: &mut Json, depth: usize) {
    if let Json::Object(schema) = node {
        normalize_custom_types(schema, depth);
    }
}
