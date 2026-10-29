use super::error::DefinitionsError;
use super::legacy::{DEFS_MAX_INLINED_BYTES, unpack_legacy_defs};
use crate::arguments::json::{Json, Object};

// `type` and `properties` are always present once sanitized; the rest carries through
// only when the caller supplied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicInputSchema {
    pub schema_type: Json,
    pub properties: Json,
    pub additional_properties: Option<Json>,
    pub required: Option<Json>,
    pub defs: Option<Json>,
    pub strict: Option<Json>,
}

const ANTHROPIC_TYPE: &str = "object";

// Legacy `definitions` / `components.schemas` references are inlined before filtering,
// since Anthropic only resolves `$defs` and would otherwise see a dangling pointer.
pub fn sanitize_for_anthropic(
    input_schema: Object,
) -> Result<AnthropicInputSchema, DefinitionsError> {
    let mut normalized = input_schema;
    let has_object_type =
        matches!(normalized.get("type"), Some(Json::Text(value)) if value == ANTHROPIC_TYPE);
    if !has_object_type {
        normalized.insert("type".to_owned(), Json::Text(ANTHROPIC_TYPE.to_owned()));
    }
    if !normalized.contains_key("properties") {
        normalized.insert("properties".to_owned(), Json::Object(Object::new()));
    }
    let expanded = unpack_legacy_defs(normalized, DEFS_MAX_INLINED_BYTES)?;
    Ok(AnthropicInputSchema {
        schema_type: take(&expanded, "type"),
        properties: take(&expanded, "properties"),
        additional_properties: expanded.get("additionalProperties").cloned(),
        required: expanded.get("required").cloned(),
        defs: expanded.get("$defs").cloned(),
        strict: expanded.get("strict").cloned(),
    })
}

fn take(schema: &Object, key: &str) -> Json {
    schema.get(key).cloned().unwrap_or(Json::Null)
}

mod tests;
