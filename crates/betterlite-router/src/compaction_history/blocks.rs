use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[must_use]
pub fn field_text<'fields>(
    fields: &'fields IndexMap<String, Json>,
    key: &str,
) -> Option<&'fields str> {
    match fields.get(key) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) | None => None,
    }
}

// LiteLLM only tests thinking_blocks for truthiness, so an empty list or a zero counts as absent
pub(crate) fn counts_as_present(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_none_or(|float| float != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
