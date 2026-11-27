use betterlite_anthropic::option_primitives::context_management::map_context_management;
use betterlite_models::decisions::Json;

use crate::api::messages_compaction::error::SpecError;

// _normalize_spec: an OpenAI list goes through B2201, a native dict keeps its edits, and only dict edits survive
pub fn normalize_context_edit_shapes(spec: Option<&Json>) -> Result<Vec<Json>, SpecError> {
    let Some(spec) = spec else {
        return Ok(Vec::new());
    };
    let normalized = match spec {
        Json::List(_) => map_context_management(spec)?.map(Json::Object),
        Json::Object(fields) => Some(Json::Object(fields.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
    };
    Ok(dict_edits(normalized.as_ref()))
}

// _edits_from: a missing, null, non-list or empty edits value leaves nothing to run
fn dict_edits(normalized: Option<&Json>) -> Vec<Json> {
    let Some(Json::Object(fields)) = normalized else {
        return Vec::new();
    };
    let Some(Json::List(entries)) = fields.get("edits") else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|entry| matches!(entry, Json::Object(_)))
        .cloned()
        .collect()
}
