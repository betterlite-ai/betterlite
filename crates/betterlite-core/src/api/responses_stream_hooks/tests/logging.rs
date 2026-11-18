use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::logging::{logging_event, restore_headers};
use super::super::state::ResponsesStreamHookState;
use super::support::{
    Cache, CacheHandler, END, START, Success, base_state, completed_event, completed_with_hidden,
    decode, event_json, hooks, json_object, path, path_mut, redactor, text, timestamp,
};

fn object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(json_object(pairs))
}

#[tokio::test]
async fn completed_logging_once() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    let handler = Arc::new(CacheHandler::new(json_object(&[(
        "stream",
        Json::Bool(true),
    )])));
    let success = Success::recording();
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(handler);
    hooks.success = Some(Arc::<Success>::clone(&success));
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    state.completed_response_cache_hit = Some(false);
    let mut callback = ResponsesStreamHookState::default();
    let details = IndexMap::new();

    hooks
        .log_completed_response(&mut state, &mut callback, &details)
        .await
        .expect("the success path logs");

    assert!(state.completed_response_logged);
    assert!(state.completed_response_cached);
    assert_eq!(cache.writes().len(), 1);
    assert!(!cache.writes()[0].asynchronous);
    assert_eq!(success.order(), vec!["async", "sync"]);
    let async_request = &success.async_calls()[0];
    assert!(!async_request.prefer_async_handlers);
    assert_eq!(async_request.cache_hit, Some(false));
    assert_eq!(async_request.start_time, Some(timestamp(START)));
    assert_eq!(async_request.end_time, Some(timestamp(END)));
    assert_eq!(
        path(async_request.result.response(), &["response", "id"]),
        Some(&text("resp_1"))
    );

    hooks
        .log_completed_response(&mut state, &mut callback, &details)
        .await
        .expect("a second call is a no-op");

    assert_eq!(cache.writes().len(), 1);
    assert_eq!(success.async_calls().len(), 1);
    assert_eq!(success.sync_calls().len(), 1);
}

#[tokio::test]
async fn completed_logging_once_restores_the_provider_headers() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let success = Success::recording();
    hooks.success = Some(Arc::<Success>::clone(&success));
    let source = object(&[
        (
            "additional_headers",
            object(&[("x-source", text("provider"))]),
        ),
        ("headers", object(&[("x-source-raw", text("raw"))])),
        ("response_cost", text("9")),
    ]);
    let mut state = base_state();
    state.completed_response = Some(completed_with_hidden(source));
    state.hidden_params.additional_headers = json_object(&[("x-iterator", text("fallback"))]);
    state.raw_response_headers = IndexMap::from([("x-iterator-raw".to_owned(), "raw".to_owned())]);
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .log_completed_response(&mut state, &mut callback, &IndexMap::new())
        .await
        .expect("the success path logs");

    let calls = success.async_calls();
    let payload = calls[0].result.response();
    assert_eq!(
        path(
            payload,
            &["response", "_hidden_params", "additional_headers"]
        ),
        Some(&object(&[("x-source", text("provider"))]))
    );
    assert_eq!(
        path(payload, &["response", "_hidden_params", "headers"]),
        Some(&object(&[("x-source-raw", text("raw"))]))
    );
    assert_eq!(
        path(payload, &["response", "_hidden_params", "response_cost"]),
        None
    );
}

#[tokio::test]
async fn completed_logging_once_skips_the_cache_when_persistence_is_off() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(Arc::new(CacheHandler::new(json_object(&[(
        "stream",
        Json::Bool(true),
    )]))));
    let success = Success::recording();
    hooks.success = Some(Arc::<Success>::clone(&success));
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    state.persist_before_logging = false;

    hooks
        .log_completed_response(
            &mut state,
            &mut ResponsesStreamHookState::default(),
            &IndexMap::new(),
        )
        .await
        .expect("the success path logs");

    assert!(cache.writes().is_empty());
    assert_eq!(success.async_calls().len(), 1);
}

#[tokio::test]
async fn completed_logging_once_passes_the_cache_hit_of_a_cached_response() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(Arc::new(CacheHandler::new(json_object(&[(
        "stream",
        Json::Bool(true),
    )]))));
    let success = Success::recording();
    hooks.success = Some(Arc::<Success>::clone(&success));
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    state.completed_response_cached = true;
    state.completed_response_cache_hit = Some(true);

    hooks
        .log_completed_response(
            &mut state,
            &mut ResponsesStreamHookState::default(),
            &IndexMap::new(),
        )
        .await
        .expect("the success path logs");

    assert!(cache.writes().is_empty());
    assert_eq!(success.async_calls()[0].cache_hit, Some(true));
}

#[tokio::test]
async fn completed_logging_once_does_not_log_again_after_a_failed_cache_write() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    hooks.cache = Some(Cache::failing("broken"));
    hooks.cache_handler = Some(Arc::new(CacheHandler::new(json_object(&[(
        "stream",
        Json::Bool(true),
    )]))));
    let success = Success::recording();
    hooks.success = Some(Arc::<Success>::clone(&success));
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    let mut callback = ResponsesStreamHookState::default();

    let rejected = hooks
        .log_completed_response(&mut state, &mut callback, &IndexMap::new())
        .await;
    assert!(rejected.is_err());
    assert!(state.completed_response_logged);
    assert!(success.async_calls().is_empty());

    hooks
        .log_completed_response(&mut state, &mut callback, &IndexMap::new())
        .await
        .expect("the latch makes the second call a no-op");

    assert!(success.async_calls().is_empty());
    assert!(success.sync_calls().is_empty());
}

#[tokio::test]
async fn completed_logging_once_schedules_the_async_dispatch() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let cache = Cache::succeeding();
    hooks.cache = Some(Arc::<Cache>::clone(&cache));
    hooks.cache_handler = Some(Arc::new(CacheHandler::new(json_object(&[(
        "stream",
        Json::Bool(true),
    )]))));
    let success = Success::recording();
    hooks.success = Some(Arc::<Success>::clone(&success));
    let mut state = base_state();
    state.completed_response = Some(completed_event());

    hooks
        .log_completed_response_async(
            &mut state,
            &mut ResponsesStreamHookState::default(),
            &IndexMap::new(),
        )
        .await
        .expect("the success path logs");

    assert!(success.dispatches().is_empty());
    state.pending_tasks.close();
    state.pending_tasks.wait().await;
    let dispatches = success.dispatches();
    assert_eq!(dispatches.len(), 1);
    assert!(dispatches[0].prefer_async_handlers);
    assert!(cache.writes()[0].asynchronous);
    assert!(state.completed_response_cached);
}

#[tokio::test]
async fn completed_logging_once_defers_an_armed_dispatch() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let success = Success::deferred();
    hooks.success = Some(Arc::<Success>::clone(&success));
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    state.persist_before_logging = false;
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .log_completed_response_async(&mut state, &mut callback, &IndexMap::new())
        .await
        .expect("the success path logs");

    assert!(success.dispatches().is_empty());
    let stored = callback
        .deferred_stream_complete_args
        .as_ref()
        .expect("the dispatch is parked");
    assert!(stored.prefer_async_handlers);
    assert_eq!(
        path(stored.result.response(), &["response", "id"]),
        Some(&text("resp_1"))
    );
}

#[test]
fn logging_header_isolation() {
    let event = completed_with_hidden(object(&[
        (
            "additional_headers",
            object(&[("x-source", text("provider"))]),
        ),
        ("headers", object(&[("x-source-raw", text("raw"))])),
        ("response_cost", text("9")),
    ]));
    let mut state = base_state();
    state.hidden_params.additional_headers = json_object(&[("x-iterator", text("fallback"))]);
    state.raw_response_headers = IndexMap::from([("x-iterator-raw".to_owned(), "raw".to_owned())]);

    let mut payload = logging_event(&event, &state.hidden_params, &state.raw_response_headers);

    assert_eq!(
        path(
            &payload,
            &["response", "_hidden_params", "additional_headers"]
        ),
        Some(&object(&[("x-source", text("provider"))]))
    );
    assert_eq!(
        path(&payload, &["response", "_hidden_params", "headers"]),
        Some(&object(&[("x-source-raw", text("raw"))]))
    );
    assert_eq!(
        path(&payload, &["response", "_hidden_params", "response_cost"]),
        None
    );
    assert_eq!(
        path(
            &event_json(&event),
            &["response", "_hidden_params", "response_cost"]
        ),
        Some(&text("9"))
    );

    let additional = path_mut(
        &mut payload,
        &["response", "_hidden_params", "additional_headers"],
    )
    .expect("the copy carries the restored headers");
    if let Json::Object(fields) = additional {
        fields.insert("x-mutated".to_owned(), text("yes"));
    }
    assert_eq!(
        path(
            &event_json(&event),
            &[
                "response",
                "_hidden_params",
                "additional_headers",
                "x-mutated"
            ]
        ),
        None
    );
    assert!(
        !state
            .hidden_params
            .additional_headers
            .contains_key("x-mutated")
    );
}

#[test]
fn logging_header_isolation_falls_back_to_the_iterator_snapshots() {
    let event = completed_event();
    let mut state = base_state();
    state.hidden_params.additional_headers = json_object(&[("x-iterator", text("fallback"))]);
    state.raw_response_headers = IndexMap::from([("x-iterator-raw".to_owned(), "raw".to_owned())]);

    let payload = logging_event(&event, &state.hidden_params, &state.raw_response_headers);

    assert_eq!(
        path(
            &payload,
            &["response", "_hidden_params", "additional_headers"]
        ),
        Some(&object(&[("x-iterator", text("fallback"))]))
    );
    assert_eq!(
        path(&payload, &["response", "_hidden_params", "headers"]),
        Some(&object(&[("x-iterator-raw", text("raw"))]))
    );
}

#[test]
fn logging_header_isolation_replaces_a_truthy_nonmapping_candidate() {
    let event = completed_with_hidden(object(&[
        ("additional_headers", text("not a mapping")),
        ("headers", Json::List(vec![Json::Null])),
    ]));
    let mut state = base_state();
    state.hidden_params.additional_headers = json_object(&[("x-iterator", text("fallback"))]);
    state.raw_response_headers = IndexMap::from([("x-iterator-raw".to_owned(), "raw".to_owned())]);

    let payload = logging_event(&event, &state.hidden_params, &state.raw_response_headers);

    assert_eq!(
        path(
            &payload,
            &["response", "_hidden_params", "additional_headers"]
        ),
        Some(&object(&[]))
    );
    assert_eq!(
        path(&payload, &["response", "_hidden_params", "headers"]),
        Some(&object(&[]))
    );
}

#[test]
fn logging_header_isolation_keeps_a_falsy_nonmapping_fallback() {
    let event = completed_with_hidden(object(&[
        ("additional_headers", Json::List(Vec::new())),
        ("headers", Json::Null),
    ]));
    let mut state = base_state();
    state.hidden_params.additional_headers = json_object(&[("x-iterator", text("fallback"))]);
    state.raw_response_headers = IndexMap::from([("x-iterator-raw".to_owned(), "raw".to_owned())]);

    let payload = logging_event(&event, &state.hidden_params, &state.raw_response_headers);

    assert_eq!(
        path(
            &payload,
            &["response", "_hidden_params", "additional_headers"]
        ),
        Some(&object(&[("x-iterator", text("fallback"))]))
    );
    assert_eq!(
        path(&payload, &["response", "_hidden_params", "headers"]),
        Some(&object(&[("x-iterator-raw", text("raw"))]))
    );
}

#[test]
fn logging_header_isolation_lets_existing_target_keys_win() {
    let mut target = completed_with_hidden(object(&[
        (
            "additional_headers",
            object(&[("x-target", text("target"))]),
        ),
        ("model_id", text("model-1")),
    ]));
    let mut state = base_state();
    state.raw_response_headers = IndexMap::from([("x-raw".to_owned(), "raw".to_owned())]);

    restore_headers(
        &mut target,
        &state.hidden_params,
        &state.raw_response_headers,
        None,
    );

    let target_json = event_json(&target);
    let hidden = path(&target_json, &["response", "_hidden_params"]).expect("restored");
    assert_eq!(
        path(hidden, &["additional_headers", "x-target"]),
        Some(&text("target"))
    );
    assert_eq!(path(hidden, &["model_id"]), Some(&text("model-1")));
    assert_eq!(path(hidden, &["headers", "x-raw"]), Some(&text("raw")));
}

#[rstest::rstest]
#[case::generic(r#"{"type":"response.queued","response":{"id":"resp_1"}}"#)]
#[case::delta(r#"{"type":"response.output_text.delta","sequence_number":1,"item_id":"item","output_index":0,"content_index":0,"delta":"hi"}"#)]
fn logging_header_isolation_ignores_events_without_a_typed_response(#[case] raw: &str) {
    let state = base_state();
    let payload = logging_event(
        &decode(raw),
        &state.hidden_params,
        &state.raw_response_headers,
    );
    assert_eq!(path(&payload, &["response", "_hidden_params"]), None);
}
