use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub(crate) const fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) const fn text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(found) => Some(found.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// Python's bool(), which decides a fix-up branch here: an empty value counts as absent
pub(crate) fn counts_as_present(value: &Json) -> bool {
    match value {
        Json::Null | Json::Bool(false) => false,
        Json::Bool(true) => true,
        Json::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        Json::Text(found) => !found.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
