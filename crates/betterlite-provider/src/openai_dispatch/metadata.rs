use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::metadata_filter_limit;
use indexmap::IndexMap;

// get_requester_metadata: a cleaned nested requester_metadata wins, and an empty one falls back to the whole map.
#[must_use]
pub fn requester_metadata(metadata: &IndexMap<String, JsonValue>) -> Option<JsonValue> {
    if metadata.is_empty() {
        return None;
    }
    if let Some(JsonValue::Object(nested)) = metadata.get("requester_metadata")
        && let Some(cleaned) = clean_metadata(nested)
    {
        return Some(cleaned);
    }
    clean_metadata(metadata)
}

// add_openai_metadata keeps string values that are not hidden_params and takes the first sixteen.
fn clean_metadata(fields: &IndexMap<String, JsonValue>) -> Option<JsonValue> {
    let raw = fields
        .iter()
        .map(|(name, value)| Some((name.clone(), serde_json::to_value(value).ok()?)))
        .collect::<Option<IndexMap<_, _>>>()?;
    let cleaned = metadata_filter_limit(Some(&raw))?;
    if cleaned.is_empty() {
        return None;
    }
    let entries = cleaned
        .into_iter()
        .map(|(name, text)| (name, JsonValue::Text(text)));
    Some(JsonValue::Object(entries.collect()))
}
