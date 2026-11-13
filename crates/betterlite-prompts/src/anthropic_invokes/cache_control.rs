use crate::arguments::json::{Json, Object};

const CACHE_CONTROL: &str = "cache_control";

#[must_use]
pub fn add_cache_control_to_content(mut element: Object, original: &Object) -> Object {
    if let Some(Json::Object(cache_control)) = original.get(CACHE_CONTROL) {
        element.insert(
            CACHE_CONTROL.to_owned(),
            Json::Object(cache_control.clone()),
        );
    }
    element
}
