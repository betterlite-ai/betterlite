//! Ordered side effects around terminal Responses stream events.
//!
//! These functions deliberately accept their effects as dependencies. The stream
//! driver owns the state and supplies the configured cache/logging/callback
//! adapters; this module owns the ordering and the once-only latches.

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

/// Whether a request was originally streamed or was converted to streaming by
/// one of LiteLLM's response interception paths.
#[must_use]
pub fn converted_stream_requested(kwargs: &IndexMap<String, Json>) -> bool {
    [
        "_websearch_interception_converted_stream",
        "_compression_interception_converted_stream",
        "_headroom_interception_converted_stream",
        "_code_interpreter_interception_converted_stream",
    ]
    .iter()
    .any(|key| truthy(kwargs.get(*key)))
}

/// Rust's typed JSON model makes truthiness explicit at this Python boundary.
fn truthy(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null | Json::Bool(false)) => false,
        Some(Json::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(fields)) => !fields.is_empty(),
        Some(Json::Bool(true)) => true,
    }
}

/// LiteLLM's request kwargs for storing a completed streamed response.
///
/// The input is copied, never mutated. A preset key takes precedence over the
/// request key, while a null preset behaves as if it were absent.
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

/// Return whether a request is eligible for a terminal streamed response write.
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
mod tests {
    use super::{cache_request_kwargs, converted_stream_requested, is_stream_cache_eligible};
    use betterlite_models::decisions::Json;
    use indexmap::IndexMap;

    #[test]
    fn cache_kwargs_copy_and_choose_preset_key() {
        let original = IndexMap::from([
            ("stream".to_owned(), Json::Bool(true)),
            (
                "custom_llm_provider".to_owned(),
                Json::Text("openai".to_owned()),
            ),
            ("metadata".to_owned(), Json::Null),
            ("cache_key".to_owned(), Json::Text("request".to_owned())),
        ]);
        let copied = cache_request_kwargs(&original, Some(&Json::Text("preset".to_owned())));
        assert_eq!(
            copied.get("cache_key"),
            Some(&Json::Text("preset".to_owned()))
        );
        assert!(!copied.contains_key("metadata"));
        assert!(!copied.contains_key("custom_llm_provider"));
        assert_eq!(
            original.get("cache_key"),
            Some(&Json::Text("request".to_owned()))
        );
    }

    #[test]
    fn converted_stream_flags_require_truthy_values() {
        let mut kwargs = IndexMap::from([(
            "_websearch_interception_converted_stream".to_owned(),
            Json::Bool(false),
        )]);
        assert!(!converted_stream_requested(&kwargs));
        kwargs.insert(
            "_code_interpreter_interception_converted_stream".to_owned(),
            Json::Bool(true),
        );
        assert!(converted_stream_requested(&kwargs));
    }

    #[test]
    fn terminal_cache_requires_completed_event_and_all_dependencies() {
        let kwargs = IndexMap::new();
        assert!(is_stream_cache_eligible(
            "response.completed",
            Some(&Json::Bool(true)),
            &kwargs,
            true,
            true,
            true,
            true
        ));
        assert!(!is_stream_cache_eligible(
            "response.failed",
            Some(&Json::Bool(true)),
            &kwargs,
            true,
            true,
            true,
            true
        ));
        assert!(!is_stream_cache_eligible(
            "response.completed",
            Some(&Json::Bool(true)),
            &kwargs,
            true,
            true,
            false,
            true
        ));
    }
}
