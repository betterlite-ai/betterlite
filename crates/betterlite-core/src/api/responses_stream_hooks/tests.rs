#![cfg(test)]

use super::{
    CacheDependencies, CacheEligibility, cache_request_kwargs, converted_stream_requested,
    is_stream_cache_eligible,
};
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
    let input = CacheEligibility {
        event_type: "response.completed",
        stream: Some(&Json::Bool(true)),
        kwargs: &kwargs,
        dependencies: CacheDependencies {
            has_typed_response: true,
            has_cache_handler: true,
        },
        global_cache: true,
        store_allowed: true,
    };
    assert!(is_stream_cache_eligible(&input));
    assert!(!is_stream_cache_eligible(&CacheEligibility {
        event_type: "response.failed",
        ..input
    }));
    assert!(!is_stream_cache_eligible(&CacheEligibility {
        has_global_cache: false,
        ..input
    }));
}
