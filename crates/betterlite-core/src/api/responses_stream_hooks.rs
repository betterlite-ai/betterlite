use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[must_use]
pub fn converted_stream_requested(kwargs: &IndexMap<String, Json>) -> bool {
    [
        "_websearch_interception_converted_stream",
        "_compression_interception_converted_stream",
        "_headroom_interception_converted_stream",
        "_code_interpreter_interception_converted_stream",
    ]
    .iter()
    .any(|key| value_is_set(kwargs.get(*key)))
}

fn value_is_set(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null | Json::Bool(false)) => false,
        Some(Json::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(fields)) => !fields.is_empty(),
        Some(Json::Bool(true)) => true,
    }
}

#[must_use]
pub fn cache_request_kwargs(
    kwargs: &IndexMap<String, Json>,
    preset_cache_key: Option<&Json>,
) -> IndexMap<String, Json> {
    let mut copied = kwargs.clone();
    copied.shift_remove("custom_llm_provider");
    if matches!(copied.get("metadata"), Some(Json::Null)) {
        copied.shift_remove("metadata");
    }
    let request_key = copied.shift_remove("cache_key");
    let selected_key = match preset_cache_key {
        Some(Json::Null) | None => request_key,
        Some(key) => Some(key.clone()),
    };
    if let Some(key) = selected_key {
        copied.insert("cache_key".to_owned(), key);
    }
    copied
}

#[must_use]
pub fn is_stream_cache_eligible(
    event_type: &str,
    stream: Option<&Json>,
    kwargs: &IndexMap<String, Json>,
    has_typed_response: bool,
    has_cache_handler: bool,
    has_global_cache: bool,
    store_allowed: bool,
) -> bool {
    event_type == "response.completed"
        && has_typed_response
        && has_cache_handler
        && has_global_cache
        && store_allowed
        && (matches!(stream, Some(Json::Bool(true))) || converted_stream_requested(kwargs))
}

#[cfg(test)]
mod tests;
