use betterlite_anthropic::option_primitives::context_management::map_context_management;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::messages_compaction::error::SpecError;

// _normalize_spec: an OpenAI list goes through B2201, a native dict keeps its edits, and only dict edits survive
pub fn normalize_context_edit_shapes(
    spec: Option<&Json>,
) -> Result<Vec<IndexMap<String, Json>>, SpecError> {
    let Some(raw) = spec else {
        return Ok(Vec::new());
    };
    let normalized = match raw {
        Json::List(_) => map_context_management(raw)?.map(Json::Object),
        Json::Object(fields) => Some(Json::Object(fields.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
    };
    Ok(dict_edits(normalized.as_ref()))
}

// _edits_from: a missing, null, non-list or empty edits value leaves nothing to run
fn dict_edits(normalized: Option<&Json>) -> Vec<IndexMap<String, Json>> {
    let Some(Json::Object(fields)) = normalized else {
        return Vec::new();
    };
    let Some(Json::List(entries)) = fields.get("edits") else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            Json::Object(edit) => Some(edit.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}
