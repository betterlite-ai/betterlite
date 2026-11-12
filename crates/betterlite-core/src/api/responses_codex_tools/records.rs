use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::CustomToolsError;

// a Python object with attributes iterates as (name, value) pairs; anything else has none
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeValue {
    Plain(Json),
    Record(Vec<Json>),
}

pub fn attribute_fields(
    value: &AttributeValue,
) -> Result<IndexMap<String, Json>, CustomToolsError> {
    let AttributeValue::Record(items) = value else {
        return Ok(IndexMap::new());
    };
    let mut fields = IndexMap::new();
    for (position, item) in items.iter().enumerate() {
        let Json::List(pair) = item else {
            return Err(CustomToolsError::NotAPair { position });
        };
        let [Json::Text(name), field] = pair.as_slice() else {
            return Err(CustomToolsError::NotAPair { position });
        };
        fields.insert(name.clone(), field.clone());
    }
    Ok(fields)
}

pub fn input_item_role(item: &IndexMap<String, Json>) -> Result<String, CustomToolsError> {
    match item.get("role") {
        Some(Json::Text(role)) if !role.is_empty() => Ok(role.clone()),
        None | Some(Json::Null | Json::Text(_)) => Ok("user".to_owned()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CustomToolsError::RoleNotText)
        }
    }
}
