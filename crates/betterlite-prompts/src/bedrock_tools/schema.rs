use crate::arguments::json::{Json, Object};
use crate::bedrock_tools::capabilities::BedrockToolCapabilities;
use crate::bedrock_tools::error::BedrockToolError;
use crate::definitions::estimate::estimate_json_bytes;
use crate::definitions::expand::unpack_defs;

const VALID_ROOT_TYPES: [&str; 7] = [
    "array", "boolean", "integer", "null", "number", "object", "string",
];
const CUSTOM_TYPE_NESTING: usize = 1024;

// One budget covers the whole tools list, so a long list cannot multiply the per-schema limit.
pub struct ExpansionBudget {
    limit: usize,
    remaining: usize,
}

impl ExpansionBudget {
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            remaining: limit,
        }
    }

    fn charge(&mut self, schema: &Json) -> Result<(), BedrockToolError> {
        let spent = estimate_json_bytes(schema);
        self.remaining = self
            .remaining
            .checked_sub(spent)
            .ok_or(BedrockToolError::SchemaBudget { limit: self.limit })?;
        Ok(())
    }
}

// Bedrock keeps only these four schema keys, so everything else a client sent is dropped before the request.
pub fn parameter_schema(
    parameters: &Json,
    capabilities: BedrockToolCapabilities,
    budget: &mut ExpansionBudget,
) -> Result<Json, BedrockToolError> {
    let Json::Object(object) = parameters else {
        return Err(BedrockToolError::WrongType {
            field: "parameters",
        });
    };
    let expanded = expand_parameters(object, budget)?;
    let mut schema = Object::new();
    schema.insert("type".to_owned(), root_type(&expanded)?);
    schema.insert(
        "properties".to_owned(),
        take(
            &expanded,
            "properties",
            ExpectedKind::Object,
            Json::Object(Object::new()),
        )?,
    );
    schema.insert(
        "required".to_owned(),
        take(
            &expanded,
            "required",
            ExpectedKind::StringList,
            Json::List(Vec::new()),
        )?,
    );
    if capabilities.supports_strict_tools
        && let Some(additional) = expanded
            .get("additionalProperties")
            .filter(|value| !matches!(value, Json::Null))
    {
        schema.insert("additionalProperties".to_owned(), additional.clone());
    }
    let projected = Json::Object(schema);
    budget.charge(&projected)?;
    Ok(projected)
}

fn expand_parameters(
    object: &Object,
    budget: &ExpansionBudget,
) -> Result<Object, BedrockToolError> {
    let mut fields = object.clone();
    let defs = match fields.shift_remove("$defs") {
        None | Some(Json::Null) => Object::new(),
        Some(Json::Object(defs)) => defs,
        Some(_) => {
            return Err(BedrockToolError::WrongType { field: "$defs" });
        }
    };
    let mut expanded = unpack_defs(fields, &defs, Some(budget.remaining))
        .map_err(|source| BedrockToolError::Schema { source })?;
    normalize_custom_types(&mut expanded, 0);
    Ok(expanded)
}

fn root_type(parameters: &Object) -> Result<Json, BedrockToolError> {
    let Some(value) = parameters.get("type") else {
        return Ok(Json::Text("object".to_owned()));
    };
    match value {
        Json::Text(name) if VALID_ROOT_TYPES.contains(&name.as_str()) => {
            Ok(Json::Text(name.clone()))
        }
        Json::Null | Json::Text(_) => Ok(Json::Text("object".to_owned())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(BedrockToolError::WrongType { field: "type" })
        }
    }
}

#[derive(Clone, Copy)]
enum ExpectedKind {
    Object,
    StringList,
}

fn take(
    parameters: &Object,
    key: &'static str,
    kind: ExpectedKind,
    default: Json,
) -> Result<Json, BedrockToolError> {
    match (parameters.get(key), kind) {
        (None | Some(Json::Null), _) => Ok(default),
        (Some(Json::Object(fields)), ExpectedKind::Object) => Ok(Json::Object(fields.clone())),
        (Some(Json::List(items)), ExpectedKind::StringList)
            if items.iter().all(|item| item.as_str().is_some()) =>
        {
            Ok(Json::List(items.clone()))
        }
        (Some(_), ExpectedKind::Object | ExpectedKind::StringList) => {
            Err(BedrockToolError::WrongType { field: key })
        }
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
