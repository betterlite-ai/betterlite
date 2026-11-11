use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

#[must_use]
pub fn jsonable_dict(value: &Json) -> Option<Object> {
    match value {
        Json::Object(fields) => Some(fields.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// The model branch of LiteLLM's adapter: a record dumps as JSON with unset fields left out.
#[must_use]
pub fn record_dict<Record>(record: &Record) -> Option<Object>
where
    Record: Serialize,
{
    let value = serde_json::to_value(record).ok()?;
    match serde_json::from_value::<Json>(value).ok()? {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub fn opaque_dict_list(value: &Json) -> Option<Vec<Object>> {
    let Json::List(items) = value else {
        return None;
    };
    let entries: Vec<Object> = items.iter().filter_map(jsonable_dict).collect();
    (!entries.is_empty()).then_some(entries)
}

mod tests;
