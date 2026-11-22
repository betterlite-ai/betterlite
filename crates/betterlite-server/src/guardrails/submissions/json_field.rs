use betterlite_pyhost::json_values::JsonObject;

use super::rows::ColumnJson;

// LiteLLM's _parse_json_field: an object as stored, or text that parses to an object, else nothing
pub(super) fn parse_json_field(column: Option<&ColumnJson>) -> Option<JsonObject> {
    match column {
        None => None,
        Some(ColumnJson::Object(map)) => Some(map.clone()),
        Some(ColumnJson::Text(text)) => serde_json::from_str::<JsonObject>(text).ok(),
    }
}
