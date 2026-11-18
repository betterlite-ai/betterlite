use std::str::FromStr as _;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use tokio::runtime::Builder;

use super::super::cache::{cache_request_kwargs, converted_stream_requested};
use super::super::ports::PortFailure;
use super::support::{
    Cache, CacheHandler, base_state, capture, completed_event, decode, dual_token, hooks,
    json_object, json_value, number, path, redactor, response_failed_event, text,
};
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;

fn streaming_handler(kwargs: &[(&str, Json)]) -> Arc<CacheHandler> {
    Arc::new(CacheHandler::new(json_object(kwargs)))
}

fn streaming_state() -> ResponsesStreamBaseState {
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    state
}

#[tokio::test]
async fn terminal_cache_write() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    let handler = streaming_handler(&[
        ("stream", Json::Bool(true)),
        ("custom_llm_provider", text("openai")),
        ("metadata", Json::Null),
        ("cache_key", text("request-key")),
    ]);
    hooks.cache = Some(cache.clone());
    hooks.cache_handler = Some(handler.clone());
    let mut state = streaming_state();

    hooks
        .persist_completed_response_to_cache(&mut state)
        .expect("the synchronous write succeeds");

    assert!(state.completed_response_cached);
    let writes = cache.writes();
    assert_eq!(writes.len(), 1);
    assert!(!writes[0].asynchronous);
    assert_eq!(
        writes[0].request_kwargs.get("cache_key"),
        Some(&text("request-key"))
    );
    assert_eq!(
        writes[0].request_kwargs.get("stream"),
        Some(&Json::Bool(true))
    );
    assert!(!writes[0].request_kwargs.contains_key("custom_llm_provider"));
    assert!(!writes[0].request_kwargs.contains_key("metadata"));
    assert!(writes[0].dynamic.is_none());
    let payload: Json = serde_json::from_str(&writes[0].payload).expect("the payload is JSON");
    assert_eq!(path(&payload, &["id"]), Some(&text("resp_1")));
    assert_eq!(
        path(&payload, &["usage", "total_tokens"]),
        Some(&number("15"))
    );
    assert_eq!(
        handler.seen(),
        vec![json_object(&[
            ("stream", Json::Bool(true)),
            ("cache_key", text("request-key")),
        ])]
    );
}

#[tokio::test]
async fn terminal_cache_write_takes_the_preset_key_over_the_request_key() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    let handler = Arc::new(
        CacheHandler::new(json_object(&[
            ("stream", Json::Bool(true)),
            ("cache_key", text("request-key")),
        ]))
        .with_preset("preset-key")
        .with_dual(dual_token()),
    );
    hooks.cache = Some(cache.clone());
    hooks.cache_handler = Some(handler.clone());
    let mut state = streaming_state();

    hooks
        .persist_completed_response_to_cache(&mut state)
        .expect("the write succeeds");

    let writes = cache.writes();
    assert_eq!(
        writes[0].request_kwargs.get("cache_key"),
        Some(&text("preset-key"))
    );
    assert!(writes[0].dynamic.is_some());
    assert_eq!(
        handler.seen()[0].get("cache_key"),
        Some(&text("preset-key"))
    );
}

#[tokio::test]
async fn terminal_cache_write_keeps_nonnull_metadata_and_drops_a_null_key() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    hooks.cache = Some(cache.clone());
    hooks.cache_handler = Some(streaming_handler(&[
        ("stream", Json::Bool(true)),
        ("metadata", json_value(&[("user_id", text("u1"))])),
        ("cache_key", Json::Null),
    ]));
    let mut state = streaming_state();

    hooks
        .persist_completed_response_to_cache(&mut state)
        .expect("the write succeeds");

    let kwargs = &cache.writes()[0].request_kwargs;
    assert_eq!(
        kwargs.get("metadata"),
        Some(&json_value(&[("user_id", text("u1"))]))
    );
    assert!(!kwargs.contains_key("cache_key"));
}

#[tokio::test]
async fn terminal_cache_write_accepts_a_truthy_converted_stream_flag() {
    let redactor = redactor();
    for value in [Json::Bool(true), number("1"), Json::Text("yes".to_owned())] {
        let mut hooks = hooks(&redactor);
        let cache = Cache::succeeding();
        hooks.cache = Some(cache.clone());
        hooks.cache_handler = Some(streaming_handler(&[
            ("stream", Json::Bool(false)),
            ("_headroom_interception_converted_stream", value),
        ]));
        let mut state = streaming_state();

        hooks
            .persist_completed_response_to_cache(&mut state)
            .expect("the write succeeds");

        assert_eq!(cache.writes().len(), 1);
        assert!(state.completed_response_cached);
    }
}

#[tokio::test]
async fn terminal_cache_write_requires_stream_exactly_true() {
    let redactor = redactor();
    for value in [
        Json::Null,
        Json::Bool(false),
        number("1"),
        Json::Text("true".to_owned()),
    ] {
        let mut hooks = hooks(&redactor);
        let cache = Cache::succeeding();
        hooks.cache = Some(cache.clone());
        hooks.cache_handler = Some(streaming_handler(&[("stream", value)]));
        let mut state = streaming_state();

        hooks
            .persist_completed_response_to_cache(&mut state)
            .expect("a skipped write is not an error");

        assert!(cache.writes().is_empty());
        assert!(!state.completed_response_cached);
    }
}

#[tokio::test]
async fn terminal_cache_write_skips_every_missing_dependency() {
    let redactor = redactor();
    let cache = Cache::succeeding();

    let mut no_handler = hooks(&redactor);
    no_handler.cache = Some(cache.clone());
    let mut state = streaming_state();
    no_handler
        .persist_completed_response_to_cache(&mut state)
        .expect("a missing handler skips the write");

    let mut no_kwargs = hooks(&redactor);
    no_kwargs.cache = Some(cache.clone());
    no_kwargs.cache_handler = Some(Arc::new(CacheHandler::without_request_kwargs()));
    no_kwargs
        .persist_completed_response_to_cache(&mut state)
        .expect("missing request kwargs skip the write");

    let mut no_cache = hooks(&redactor);
    no_cache.cache_handler = Some(streaming_handler(&[("stream", Json::Bool(true))]));
    no_cache
        .persist_completed_response_to_cache(&mut state)
        .expect("a missing global cache skips the write");

    let mut refuses = hooks(&redactor);
    refuses.cache = Some(cache.clone());
    refuses.cache_handler = Some(Arc::new(
        CacheHandler::new(json_object(&[("stream", Json::Bool(true))])).with_store(false),
    ));
    refuses
        .persist_completed_response_to_cache(&mut state)
        .expect("a refused store skips the write");

    let mut cached = hooks(&redactor);
    cached.cache = Some(cache.clone());
    let handler = streaming_handler(&[("stream", Json::Bool(true))]);
    cached.cache_handler = Some(handler.clone());
    let mut cached_state = streaming_state();
    cached_state.completed_response_cached = true;
    cached
        .persist_completed_response_to_cache(&mut cached_state)
        .expect("an already cached response skips the write");

    assert!(cache.writes().is_empty());
    assert!(handler.seen().is_empty());
    assert!(!state.completed_response_cached);
    assert!(cached_state.completed_response_cached);
}

#[tokio::test]
async fn terminal_cache_write_requires_a_completed_typed_response() {
    let redactor = redactor();
    let cache = Cache::succeeding();
    let mut hooks = hooks(&redactor);
    hooks.cache = Some(cache.clone());
    hooks.cache_handler = Some(streaming_handler(&[("stream", Json::Bool(true))]));
    let in_progress = decode(
        r#"{"type":"response.in_progress","sequence_number":1,"response":{"id":"resp_1","created_at":1700000000,"model":"gpt-4o","output":[]}}"#,
    );
    let generic = decode(r#"{"type":"response.completed"}"#);
    for event in [in_progress, generic, response_failed_event(None, None)] {
        let mut state = base_state();
        state.completed_response = Some(event);
        hooks
            .persist_completed_response_to_cache(&mut state)
            .expect("a non terminal event skips the write");
        assert!(!state.completed_response_cached);
    }
    let mut empty = base_state();
    hooks
        .persist_completed_response_to_cache(&mut empty)
        .expect("a missing event skips the write");
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn terminal_cache_write_stays_retryable_after_a_failed_write() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let failing = Cache::failing("broken");
    hooks.cache = Some(failing.clone());
    hooks.cache_handler = Some(streaming_handler(&[("stream", Json::Bool(true))]));
    let mut state = streaming_state();

    let rejected = hooks.persist_completed_response_to_cache(&mut state);
    assert_eq!(
        rejected,
        Err(PortFailure {
            message: "broken".to_owned(),
        })
    );
    assert!(!state.completed_response_cached);
    assert_eq!(failing.writes().len(), 1);

    let succeeding = Cache::succeeding();
    hooks.cache = Some(succeeding.clone());
    hooks
        .persist_completed_response_to_cache(&mut state)
        .expect("a retry succeeds");
    assert!(state.completed_response_cached);
    assert_eq!(succeeding.writes().len(), 1);
}

#[test]
fn terminal_cache_write_logs_an_async_failure_and_keeps_the_flag() {
    let redactor = redactor();
    let cache = Cache::failing("broken");
    let (scheduled, logs) = capture(|| {
        let mut hooks = hooks(&redactor);
        hooks.cache = Some(cache.clone());
        hooks.cache_handler = Some(streaming_handler(&[("stream", Json::Bool(true))]));
        let mut state = streaming_state();
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the runtime builds");
        runtime.block_on(async {
            hooks.schedule_completed_response_cache_write(&mut state)?;
            state.pending_tasks.close();
            state.pending_tasks.wait().await;
            Ok::<bool, PortFailure>(state.completed_response_cached)
        })
    });
    assert_eq!(scheduled, Ok(true));
    assert_eq!(cache.writes().len(), 1);
    assert!(cache.writes()[0].asynchronous);
    assert!(
        logs.messages()
            .iter()
            .any(|line| line.contains("Responses stream cache write failed: broken"))
    );
}

#[test]
fn converted_stream_flags_require_truthy_values() {
    let key = "_websearch_interception_converted_stream";
    let mut kwargs = IndexMap::new();
    for value in [
        Json::Null,
        Json::Bool(false),
        number("0"),
        Json::Number(Number::from_f64(0.0).expect("a finite number")),
        Json::Text(String::new()),
        Json::List(Vec::new()),
        Json::Object(IndexMap::new()),
    ] {
        kwargs.insert(key.to_owned(), value);
        assert!(!converted_stream_requested(&kwargs));
    }
    for value in [
        Json::Bool(true),
        number("1"),
        Json::Number(Number::from_f64(0.5).expect("a finite number")),
        Json::Text("yes".to_owned()),
        Json::List(vec![Json::Null]),
        Json::Object(IndexMap::from([("key".to_owned(), Json::Null)])),
    ] {
        kwargs.insert(key.to_owned(), value);
        assert!(converted_stream_requested(&kwargs));
    }
}

#[test]
fn cache_request_kwargs_choose_the_preset_and_copy_the_input() {
    let original = json_object(&[
        ("stream", Json::Bool(true)),
        ("custom_llm_provider", text("openai")),
        ("metadata", Json::Null),
        ("cache_key", text("request")),
    ]);
    let copied = cache_request_kwargs(&original, Some("preset"));
    assert_eq!(copied.get("cache_key"), Some(&text("preset")));
    assert!(!copied.contains_key("metadata"));
    assert!(!copied.contains_key("custom_llm_provider"));
    assert_eq!(original.get("cache_key"), Some(&text("request")));
    assert_eq!(original.get("metadata"), Some(&Json::Null));

    let without_preset = cache_request_kwargs(&original, None);
    assert_eq!(without_preset.get("cache_key"), Some(&text("request")));

    let null_key = json_object(&[("cache_key", Json::Null)]);
    assert!(!cache_request_kwargs(&null_key, None).contains_key("cache_key"));
}
