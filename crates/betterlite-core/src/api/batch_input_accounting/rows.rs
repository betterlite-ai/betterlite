use betterlite_models::decisions::Json;

use crate::api::request_destination::RequestData;

// _count_entry_tokens checks these in order and counts the first one Python calls truthy
const COUNTABLE_FIELDS: [&str; 3] = ["messages", "prompt", "input"];

// What a row's token count would measure: nothing, the entry LiteLLM's counter reads, or an unreadable body
pub(super) enum RowInput {
    Zero,
    Unmeasurable,
    Measured(RequestData),
}

// Python truthiness of a JSON value: null, false, zero, an empty string, list or object are falsy
pub(super) fn counts_as_value(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|value| value.abs() > 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

// (entry.get("body") or {}).get("model"): only a non-empty string names a model the file must be checked for
pub(super) fn entry_model(entry: &RequestData) -> Option<&str> {
    let Some(Json::Object(body)) = entry.get("body") else {
        return None;
    };
    match body.get("model") {
        Some(Json::Text(model)) => (!model.is_empty()).then_some(model.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// _count_entry_tokens over one row: a falsy body counts zero, a truthy body that is not an object aborts it
pub(super) fn countable_entry(entry: &RequestData) -> RowInput {
    let body = match entry.get("body") {
        Some(Json::Object(body)) => body,
        Some(other) => {
            return if counts_as_value(other) {
                RowInput::Unmeasurable
            } else {
                RowInput::Zero
            };
        }
        None => return RowInput::Zero,
    };
    let Some((name, value)) = COUNTABLE_FIELDS.iter().find_map(|name| {
        body.get(*name)
            .filter(|value| counts_as_value(value))
            .map(|value| (*name, value))
    }) else {
        return RowInput::Zero;
    };
    RowInput::Measured(measured_entry(body, name, value))
}

// The entry the counter reads: the row's body keeps only its model and the field that counts
fn measured_entry(body: &RequestData, name: &str, value: &Json) -> RequestData {
    let mut measured_body = RequestData::new();
    if let Some(model) = body.get("model") {
        measured_body.insert("model".to_owned(), model.clone());
    }
    measured_body.insert(name.to_owned(), value.clone());
    let mut entry = RequestData::new();
    entry.insert("body".to_owned(), Json::Object(measured_body));
    entry
}
