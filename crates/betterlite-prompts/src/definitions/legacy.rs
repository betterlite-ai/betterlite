use super::error::DefinitionsError;
use super::expand::unpack_defs;
use crate::arguments::json::{Json, Object};

// Real MCP and `OpenAPI` tool schemas inline well under 1 MB, so 10 MB rejects a request-supplied schema bomb long before it becomes memory pressure.
pub const DEFS_MAX_INLINED_BYTES: usize = 10_000_000;

#[must_use]
pub fn has_legacy_defs(schema: &Json) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    object.contains_key("definitions") || components_schemas(object).is_some()
}

fn components_schemas(schema: &Object) -> Option<&Object> {
    match schema.get("components")?.as_object()?.get("schemas")? {
        Json::Object(schemas) => Some(schemas),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// `definitions` wins on a name shared with `components.schemas`: it is the JSON-Schema native namespace, and `unpack_defs` keys every reference by its last path segment, so one name can only ever resolve to one body.
pub fn unpack_legacy_defs(
    mut schema: Object,
    max_inlined_bytes: usize,
) -> Result<Object, DefinitionsError> {
    if !schema.contains_key("definitions") && components_schemas(&schema).is_none() {
        return Ok(schema);
    }
    let mut defs = match schema.shift_remove("components") {
        Some(Json::Object(components)) => match components.get("schemas") {
            Some(Json::Object(schemas)) => schemas.clone(),
            _ => Object::new(),
        },
        _ => Object::new(),
    };
    if let Some(Json::Object(definitions)) = schema.shift_remove("definitions") {
        for (key, value) in definitions {
            defs.insert(key, value);
        }
    }
    unpack_defs(schema, &defs, Some(max_inlined_bytes))
}

mod tests;
