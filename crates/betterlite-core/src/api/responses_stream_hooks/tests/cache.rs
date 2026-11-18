use super::super::ports::Dispatch;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::runtime::Builder;

use super::super::cache::{cache_request_kwargs, converted_stream_requested};
use super::super::ports::PortFailure;
use super::support::{
    Cache, CacheHandler, base_state, capture, completed_event, decode, dual_token, hooks,
    json_object, json_value, number, path, redactor, text,
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
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(Arc::<CacheHandler>::clone(&handler));
    let mut state = streaming_state();

    hooks
        .persist_cache(&mut state, Dispatch::Inline)
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
    let dual = dual_token();
    assert!(dual != dual_token());
    let handler = Arc::new(
        CacheHandler::new(json_object(&[
            ("stream", Json::Bool(true)),
            ("cache_key", text("request-key")),
        ]))
        .with_preset("preset-key")
        .with_dual(dual.clone()),
    );
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(Arc::<CacheHandler>::clone(&handler));
    let mut state = streaming_state();

    hooks
        .persist_cache(&mut state, Dispatch::Inline)
        .expect("the write succeeds");

    let writes = cache.writes();
    assert_eq!(
        writes[0].request_kwargs.get("cache_key"),
        Some(&text("preset-key"))
    );
    assert!(
        writes[0]
            .dynamic
            .as_ref()
            .is_some_and(|actual| *actual == dual)
    );
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
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(streaming_handler(&[
        ("stream", Json::Bool(true)),
        ("metadata", json_value(&[("user_id", text("u1"))])),
        ("cache_key", Json::Null),
    ]));
    let mut state = streaming_state();

    hooks
        .persist_cache(&mut state, Dispatch::Inline)
        .expect("the write succeeds");

    let kwargs = &cache.writes()[0].request_kwargs;
    assert_eq!(
        kwargs.get("metadata"),
        Some(&json_value(&[("user_id", text("u1"))]))
    );
    assert!(!kwargs.contains_key("cache_key"));
}

#[rstest::rstest]
#[case::boolean(Json::Bool(true))]
#[case::number(number("1"))]
#[case::text(Json::Text("yes".to_owned()))]
#[tokio::test]
async fn terminal_cache_write_accepts_a_truthy_converted_stream_flag(#[case] value: Json) {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(streaming_handler(&[
        ("stream", Json::Bool(false)),
        ("_headroom_interception_converted_stream", value),
    ]));
    let mut state = streaming_state();

    hooks
        .persist_cache(&mut state, Dispatch::Inline)
        .expect("the write succeeds");

    assert_eq!(cache.writes().len(), 1);
    assert!(state.completed_response_cached);
}

#[rstest::rstest]
#[case::null(Json::Null)]
#[case::disabled(Json::Bool(false))]
#[case::number(number("1"))]
#[case::text(Json::Text("true".to_owned()))]
#[tokio::test]
async fn terminal_cache_write_requires_stream_exactly_true(#[case] value: Json) {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(streaming_handler(&[("stream", value)]));
    let mut state = streaming_state();

    hooks
        .persist_cache(&mut state, Dispatch::Inline)
        .expect("a skipped write is not an error");

    assert!(cache.writes().is_empty());
    assert!(!state.completed_response_cached);
}

#[tokio::test]
async fn terminal_cache_write_skips_every_missing_dependency() {
    let redactor = redactor();
    let cache = Cache::succeeding();

    let mut no_handler = hooks(&redactor);
    no_handler.cache = Some(Arc::<Cache>::clone(&cache));
    let mut state = streaming_state();
    no_handler
        .persist_cache(&mut state, Dispatch::Inline)
        .expect("a missing handler skips the write");

    let mut no_kwargs = hooks(&redactor);
    no_kwargs.cache = Some(Arc::<Cache>::clone(&cache));
    no_kwargs.cache_handler = Some(Arc::new(CacheHandler::without_request_kwargs()));
    no_kwargs
        .persist_cache(&mut state, Dispatch::Inline)
        .expect("missing request kwargs skip the write");

    let mut no_cache = hooks(&redactor);
    no_cache.cache_handler = Some(streaming_handler(&[("stream", Json::Bool(true))]));
    no_cache
        .persist_cache(&mut state, Dispatch::Inline)
        .expect("a missing global cache skips the write");

    let mut refuses = hooks(&redactor);
    refuses.cache = Some(Arc::<Cache>::clone(&cache));
    refuses.cache_handler = Some(Arc::new(
        CacheHandler::new(json_object(&[("stream", Json::Bool(true))])).with_store(false),
    ));
    refuses
        .persist_cache(&mut state, Dispatch::Inline)
        .expect("a refused store skips the write");

    let mut cached = hooks(&redactor);
    cached.cache = Some(Arc::<Cache>::clone(&cache));
    let handler = streaming_handler(&[("stream", Json::Bool(true))]);
    cached.cache_handler = Some(Arc::<CacheHandler>::clone(&handler));
    let mut cached_state = streaming_state();
    cached_state.completed_response_cached = true;
    cached
        .persist_cache(&mut cached_state, Dispatch::Inline)
        .expect("an already cached response skips the write");

    assert!(cache.writes().is_empty());
    assert!(handler.seen().is_empty());
    assert!(!state.completed_response_cached);
    assert!(cached_state.completed_response_cached);
}

#[rstest::rstest]
#[case::progress(r#"{"type":"response.in_progress","sequence_number":1,"response":{"id":"resp_1","created_at":1700000000,"model":"gpt-4o","output":[]}}"#)]
#[case::untyped(r#"{"type":"response.completed"}"#)]
#[case::failed(r#"{"type":"response.failed","sequence_number":2,"response":{"id":"resp_1","created_at":1700000000,"model":"gpt-4o","output":[]}}"#)]
#[tokio::test]
async fn terminal_cache_write_requires_a_completed_typed_response(#[case] raw: &str) {
    let redactor = redactor();
    let cache = Cache::succeeding();
    let mut hooks = hooks(&redactor);
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(streaming_handler(&[("stream", Json::Bool(true))]));
    let mut state = base_state();
    state.completed_response = Some(decode(raw));
    hooks.persist_cache(&mut state, Dispatch::Inline).unwrap();
    assert!(!state.completed_response_cached);
    let mut empty = base_state();
    hooks.persist_cache(&mut empty, Dispatch::Inline).unwrap();
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn terminal_cache_write_stays_retryable_after_a_failed_write() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let failing = Cache::failing("broken");
    hooks.cache = Some(Arc::<Cache>::clone(&failing));
    hooks.cache_handler = Some(streaming_handler(&[("stream", Json::Bool(true))]));
    let mut state = streaming_state();

    let rejected = hooks.persist_cache(&mut state, Dispatch::Inline);
    assert_eq!(
        rejected,
        Err(PortFailure {
            message: "broken".to_owned(),
        })
    );
    assert!(!state.completed_response_cached);
    assert_eq!(failing.writes().len(), 1);

    let succeeding = Cache::succeeding();
    hooks.cache = Some(Arc::<Cache>::clone(&succeeding));
    hooks
        .persist_cache(&mut state, Dispatch::Inline)
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
        hooks.cache = Some(Arc::<Cache>::clone(&cache));
        hooks.cache_handler = Some(streaming_handler(&[("stream", Json::Bool(true))]));
        let mut state = streaming_state();
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the runtime builds");
        runtime.block_on(async {
            hooks.persist_cache(&mut state, Dispatch::Spawned)?;
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

#[rstest::rstest]
#[case::null(r#"null"#, false)]
#[case::disabled(r#"false"#, false)]
#[case::zero(r#"0"#, false)]
#[case::decimal_zero(r#"0.0"#, false)]
#[case::empty_text(r#""""#, false)]
#[case::empty_list(r#"[]"#, false)]
#[case::empty_object(r#"{}"#, false)]
#[case::enabled(r#"true"#, true)]
#[case::one(r#"1"#, true)]
#[case::fraction(r#"0.5"#, true)]
#[case::text(r#""yes""#, true)]
#[case::list(r#"[null]"#, true)]
#[case::object(r#"{"key":null}"#, true)]
#[case::negative_exponent(r#"1e-9"#, true)]
#[case::exponent_zero(r#"0e8"#, false)]
fn converted_stream_flags_require_truthy_values(#[case] raw: &str, #[case] expected: bool) {
    let value = serde_json::from_str::<Json>(raw).unwrap();
    let kwargs = IndexMap::from([("_websearch_interception_converted_stream".to_owned(), value)]);
    assert_eq!(converted_stream_requested(&kwargs), expected);
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
