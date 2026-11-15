use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub type Params = IndexMap<String, Json>;

#[must_use]
pub const fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::Text(_) => "string",
        Json::List(_) => "array",
        Json::Object(_) => "object",
    }
}

#[must_use]
pub fn text_param<'params>(params: &'params Params, key: &str) -> Option<&'params str> {
    match params.get(key) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
