use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// Builds a new messages list rather than mutating a shared source in place; returns None when there is no system prompt or the messages shape is unsupported.
#[must_use]
pub fn prepend_system_prompt(messages: Option<&Json>, system: Option<&Json>) -> Option<Vec<Json>> {
    let system_prompt = system.filter(|value| has_meaningful_value(value))?;
    let scaffold = Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("system".to_owned())),
        ("content".to_owned(), system_prompt.clone()),
    ]));
    match messages {
        Some(Json::List(items)) => {
            let mut combined = Vec::with_capacity(items.len().saturating_add(1));
            combined.push(scaffold);
            combined.extend(items.iter().cloned());
            Some(combined)
        }
        Some(value @ (Json::Object(_) | Json::Text(_))) => Some(vec![scaffold, value.clone()]),
        _ => None,
    }
}

fn has_meaningful_value(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|number_value| number_value != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
