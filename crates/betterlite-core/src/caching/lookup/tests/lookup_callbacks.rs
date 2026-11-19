use std::time::Duration;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::replay::Replayed;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_models::decisions::Json;

use super::support::{
    Fixture, MODEL, Model, Stamp, Tokens, Wrap, async_cache_lookup, cached_chat, chat_kwargs,
    computed_key, handler_named, lookup, perf_clock, services, sync_cache_lookup, worker,
};
use crate::caching::hit_callbacks::context::CACHING_DETAILS;
use crate::caching::lookup::async_lookup::async_get_cache;
use crate::caching::lookup::sync_lookup::sync_get_cache;

fn streaming_kwargs() -> KeyObject {
    let mut request = chat_kwargs();
    request.insert("stream".to_owned(), KeyValue::Bool(true));
    request
}

fn stream_model(response: Option<Replayed<Model>>) -> Model {
    let Some(Replayed::Stream(model)) = response else {
        panic!("the cached chat response replays as a stream");
    };
    model
}

fn assert_stream_wrap(fixture: &Fixture, asynchronous: bool) {
    assert_eq!(
        fixture.conversions.recorded().wraps,
        [Wrap {
            asynchronous,
            model: MODEL.to_owned(),
        }]
    );
}

#[tokio::test]
async fn an_async_hit_queues_the_converted_result() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    let key = computed_key(&fixture.cache, &request_kwargs);
    fixture.backend.seed(&key, cached_chat());
    let mut handler = handler_named(
        "acompletion",
        &["model", "messages"],
        request_kwargs.clone(),
    );
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let outcome = async_get_cache(
        &mut handler,
        &lookup(CachingSupportedCallType::Acompletion, request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the async lookup runs");
    let report = worker.flush().await;
    assert_eq!(report.completed, 1);
    assert!(outcome.logging.is_none());
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "async"]);
    let snapshots = fixture.snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].logged_response, Json::Bool(true));
}

#[tokio::test]
async fn an_async_stream_hit_defers_the_callbacks_to_the_caller() {
    let fixture = Fixture::new();
    let request_kwargs = streaming_kwargs();
    let key = computed_key(&fixture.cache, &request_kwargs);
    fixture.backend.seed(&key, cached_chat());
    let mut handler = handler_named(
        "acompletion",
        &["model", "messages"],
        request_kwargs.clone(),
    );
    let request = lookup(CachingSupportedCallType::Acompletion, request_kwargs);
    let tokens = Tokens::default();
    let (outcome, worker) = async_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    )
    .await;
    let logging = outcome
        .logging
        .expect("the stream hit keeps the logging object");
    assert!(
        logging
            .state
            .model_call_details
            .contains_key(CACHING_DETAILS)
    );
    assert!(fixture.calls.events().is_empty());
    assert_eq!(worker.stats().accepted, 0);
    let response = outcome.response.expect("a hit returns a response");
    let model = stream_model(response.cached_result);
    assert_eq!(model.stamp, Stamp::Key(key));
    assert_stream_wrap(&fixture, true);
}

#[tokio::test]
async fn a_sync_hit_dispatches_the_sync_callbacks() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    let key = computed_key(&fixture.cache, &request_kwargs);
    fixture.backend.seed(&key, cached_chat());
    let mut handler = handler_named("completion", &["model", "messages"], request_kwargs.clone());
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let mut logging = fixture.hit_logging();
    logging.state.dynamic_callbacks.success = Some(vec!["legacy_hook".to_owned()]);
    let response = sync_get_cache(
        &mut handler,
        &lookup(CachingSupportedCallType::Completion, request_kwargs),
        &mut logging,
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the sync lookup runs");
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "sync"]);
    assert_eq!(
        logging
            .state
            .model_call_details
            .get("has_logged_sync_success"),
        Some(&Json::Bool(true))
    );
    let Some(Replayed::Response(_, model)) = response.cached_result else {
        panic!("the cached chat response is converted");
    };
    assert!(model.cache_hit);
}

#[tokio::test]
async fn a_sync_hit_without_a_legacy_callable_builds_no_payload() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    let key = computed_key(&fixture.cache, &request_kwargs);
    fixture.backend.seed(&key, cached_chat());
    let mut handler = handler_named("completion", &["model", "messages"], request_kwargs.clone());
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let mut logging = fixture.hit_logging();
    sync_get_cache(
        &mut handler,
        &lookup(CachingSupportedCallType::Completion, request_kwargs),
        &mut logging,
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the sync lookup runs");
    assert!(fixture.calls.events().is_empty());
    assert!(fixture.snapshots().is_empty());
    assert!(
        logging
            .state
            .model_call_details
            .contains_key(CACHING_DETAILS)
    );
}

#[tokio::test]
async fn a_sync_stream_hit_defers_the_callbacks_to_the_caller() {
    let fixture = Fixture::new();
    let request_kwargs = streaming_kwargs();
    let key = computed_key(&fixture.cache, &request_kwargs);
    fixture.backend.seed(&key, cached_chat());
    let mut handler = handler_named("completion", &["model", "messages"], request_kwargs.clone());
    let request = lookup(CachingSupportedCallType::Completion, request_kwargs);
    let mut logging = fixture.hit_logging();
    logging.state.dynamic_callbacks.success = Some(vec!["legacy_hook".to_owned()]);
    let response = sync_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
        &mut logging,
    )
    .await;
    assert!(fixture.calls.events().is_empty());
    assert!(fixture.snapshots().is_empty());
    assert!(
        logging
            .state
            .model_call_details
            .contains_key(CACHING_DETAILS)
    );
    assert!(
        !logging
            .state
            .model_call_details
            .contains_key("has_logged_sync_success")
    );
    assert_stream_wrap(&fixture, false);
    let model = stream_model(response.cached_result);
    assert!(model.cache_hit);
}
