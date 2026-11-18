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
        Some(Json::Number(value)) => {
            value.as_i64().is_some_and(|integer| integer != 0)
                || value.as_u64().is_some_and(|integer| integer != 0)
                || value.as_f64().is_some_and(|float| float != 0.0_f64)
        }
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

#[derive(Clone, Copy)]
pub struct CacheEligibility<'a> {
    pub event_type: &'a str,
    pub stream: Option<&'a Json>,
    pub kwargs: &'a IndexMap<String, Json>,
    pub has_typed_response: bool,
    pub has_cache_handler: bool,
    pub has_global_cache: bool,
    pub store_allowed: bool,
}

#[must_use]
pub fn is_stream_cache_eligible(input: &CacheEligibility<'_>) -> bool {
    input.event_type == "response.completed"
        && input.has_typed_response
        && input.has_cache_handler
        && input.has_global_cache
        && input.store_allowed
        && (matches!(input.stream, Some(Json::Bool(true)))
            || converted_stream_requested(input.kwargs))
}

mod tests;
