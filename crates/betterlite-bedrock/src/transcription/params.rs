use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub type Params = IndexMap<String, Json>;

pub fn text_param<'params>(params: &'params Params, key: &str) -> Option<&'params str> {
    match params.get(key) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
