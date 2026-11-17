use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::member::BudgetLimitEntry;
use num_traits::Zero as _;

use super::decode::to_object;
use super::error::DecodeError;

// Falsey stored values are no windows, a string is JSON text, and every entry must be an object.
pub fn stored_budget_limit_windows(
    stored: Option<&JsonData>,
) -> Result<Vec<BTreeMap<String, JsonData>>, DecodeError> {
    let items: Vec<JsonData> = match stored {
        Some(JsonData::Array(items)) if items.is_empty() => return Ok(Vec::new()),
        Some(JsonData::Array(items)) => items.clone(),
        Some(JsonData::String(text)) if text.is_empty() => return Ok(Vec::new()),
        Some(JsonData::String(text)) => match serde_json::from_str::<JsonData>(text) {
            Ok(JsonData::Array(items)) => items,
            Ok(_) => return Err(DecodeError::StoredWindowsNotAList),
            Err(source) => return Err(DecodeError::StoredWindowsJson(source)),
        },
        Some(JsonData::Object(fields)) if fields.is_empty() => return Ok(Vec::new()),
        Some(JsonData::Object(_)) => return Err(DecodeError::StoredWindowsNotAList),
        Some(JsonData::Number(number)) if number.as_f64().is_some_and(|value| value.is_zero()) => {
            return Ok(Vec::new());
        }
        Some(JsonData::Number(_) | JsonData::Bool(true)) => {
            return Err(DecodeError::StoredWindowsNotAList);
        }
        Some(JsonData::Bool(false) | JsonData::Null) | None => return Ok(Vec::new()),
    };
    let mut windows = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let JsonData::Object(fields) = item else {
            return Err(DecodeError::StoredWindowsUnsupported { index });
        };
        windows.push(fields);
    }
    Ok(windows)
}

// A caller that validated the column through a pydantic model hands over typed entries; its dump is the window.
pub fn typed_budget_limit_windows(
    entries: Option<&[BudgetLimitEntry]>,
) -> Result<Vec<BTreeMap<String, JsonData>>, DecodeError> {
    let Some(entries) = entries else {
        return Ok(Vec::new());
    };
    entries.iter().map(to_object).collect()
}
