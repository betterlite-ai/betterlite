use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json value")
}

pub(super) fn thinking_object(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("thinking object")
}
