use super::truthiness::is_set;
use betterlite_db::budget_permission_rows::JsonData;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;

pub type Metadata = BTreeMap<String, JsonData>;

pub enum Directory<T> {
    Absent,
    Parsed(T),
    Malformed,
}

// serde reads a JSON array into a struct field by field, so the shape is checked before the decode
#[must_use]
pub const fn is_object(value: &JsonData) -> bool {
    match value {
        JsonData::Object(_) => true,
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => false,
    }
}

#[must_use]
pub fn is_attribute_list(value: &JsonData) -> bool {
    match value {
        JsonData::Array(items) => items
            .iter()
            .all(|item| matches!(item, JsonData::String(_) | JsonData::Object(_))),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Object(_) => false,
    }
}

pub fn parse_directory_metadata<T: DeserializeOwned>(
    metadata: &Metadata,
    key: &str,
    well_formed: fn(&JsonData) -> bool,
) -> Result<Directory<T>, serde_json::Error> {
    let Some(stored) = metadata.get(key).filter(|stored| is_set(stored)) else {
        return Ok(Directory::Absent);
    };
    if !well_formed(stored) {
        return Ok(Directory::Malformed);
    }
    let text = serde_json::to_string(stored)?;
    Ok(serde_json::from_str(&text).map_or(Directory::Malformed, Directory::Parsed))
}
