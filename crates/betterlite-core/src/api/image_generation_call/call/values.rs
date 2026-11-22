use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use super::CallError;

pub(super) fn header_map(
    value: &Json,
    field: &'static str,
) -> Result<Option<BTreeMap<String, String>>, CallError> {
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let Json::Object(fields) = value else {
        return Err(CallError::FieldHeaders { field });
    };
    let mut mapped = BTreeMap::new();
    for (name, entry) in fields {
        let Json::Text(text) = entry else {
            return Err(CallError::FieldHeaders { field });
        };
        mapped.insert(name.clone(), text.clone());
    }
    Ok(Some(mapped))
}

pub(super) fn string_list(value: &Json, field: &'static str) -> Result<Vec<String>, CallError> {
    let Json::List(items) = value else {
        return Err(CallError::FieldList { field });
    };
    items
        .iter()
        .map(|item| match item {
            Json::Text(text) => Ok(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(CallError::FieldList { field })
            }
        })
        .collect()
}
