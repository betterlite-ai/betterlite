use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::safe_json::safe_json_loads;

// Stored metadata and model aliases may arrive as JSON text; B192's decoder bounds the bytes and the depth.
pub fn decode_team_admin_object(text: &str) -> BTreeMap<String, JsonData> {
    let Ok(value) = safe_json_loads(text.as_bytes()) else {
        return BTreeMap::new();
    };
    let Ok(JsonData::Object(fields)) = serde_json::from_value::<JsonData>(value) else {
        return BTreeMap::new();
    };
    fields
}
