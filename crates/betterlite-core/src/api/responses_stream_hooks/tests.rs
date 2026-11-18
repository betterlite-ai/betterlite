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
fn converted_stream_values_cover_json_shapes() {
    let key = "_websearch_interception_converted_stream";
    let mut kwargs = IndexMap::new();
    for value in [
        Json::Null,
        Json::Bool(false),
        Json::Number(0.into()),
        Json::Text(String::new()),
        Json::List(Vec::new()),
        Json::Object(IndexMap::new()),
    ] {
        kwargs.insert(key.to_owned(), value);
        assert!(!converted_stream_requested(&kwargs));
    }
    for value in [
        Json::Bool(true),
        Json::Number(1.into()),
        Json::Number(serde_json::Number::from_f64(0.5).expect("finite number")),
        Json::Text("yes".to_owned()),
        Json::List(vec![Json::Null]),
        Json::Object(IndexMap::from([("key".to_owned(), Json::Null)])),
    ] {
        kwargs.insert(key.to_owned(), value);
        assert!(converted_stream_requested(&kwargs));
    }
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
        global_cache: false,
        ..input
    }));
    assert!(!is_stream_cache_eligible(&CacheEligibility {
        dependencies: CacheDependencies {
            has_typed_response: false,
            ..input.dependencies
        },
        ..input
    }));
    assert!(!is_stream_cache_eligible(&CacheEligibility {
        dependencies: CacheDependencies {
            has_cache_handler: false,
            ..input.dependencies
        },
        ..input
    }));
    assert!(!is_stream_cache_eligible(&CacheEligibility {
        store_allowed: false,
        ..input
    }));
    let false_stream = Json::Bool(false);
    assert!(!is_stream_cache_eligible(&CacheEligibility {
        stream: Some(&false_stream),
        ..input
    }));
}
