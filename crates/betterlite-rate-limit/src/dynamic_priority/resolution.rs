use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

mod tests;

const PRIORITY_FIELD: &str = "priority";

// priority_from_key_dict: a team priority takes precedence, a null one leaves the key's own priority to answer, and anything but text names no pool
#[must_use]
pub fn priority_from_key_dict<'metadata>(
    team_metadata: Option<&'metadata BTreeMap<String, JsonData>>,
    key_metadata: &'metadata BTreeMap<String, JsonData>,
) -> Option<&'metadata str> {
    let team_priority = team_metadata
        .and_then(|metadata| metadata.get(PRIORITY_FIELD))
        .filter(|value| **value != JsonData::Null);
    let chosen = team_priority.or_else(|| key_metadata.get(PRIORITY_FIELD));
    chosen.and_then(text_priority)
}

fn text_priority(value: &JsonData) -> Option<&str> {
    match value {
        JsonData::String(text) => Some(text),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}
