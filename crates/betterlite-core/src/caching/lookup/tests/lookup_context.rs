use std::time::Duration;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::replay::{Replayed, ResponseKind};
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_models::decisions::Json;

use super::support::{
    Fixture, MODEL, Metadata, Tokens, cached_chat, cached_json, chat_kwargs, computed_key,
    details_fields, duration_field, handler_named, hit_field, lookup, object, perf_clock, services,
    text, worker,
};
use crate::caching::lookup::async_lookup::async_get_cache;
use crate::caching::lookup::error::HitError;
use crate::caching::lookup::hit::{
    CacheTiming, HitKind, cache_duration, hit_cache_key, hit_record, resolve_provider,
};
use crate::caching::lookup::parent_span::add_parent_otel_span;
use crate::caching::lookup::sync_lookup::sync_get_cache;

#[tokio::test]
async fn an_async_hit_records_the_resolved_model_and_cache_duration() {
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
        perf_clock(&[Duration::from_millis(100), Duration::from_millis(175)]),
    );
    let outcome = async_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Acompletion, request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the async lookup runs");
    worker.flush().await;
    let snapshots = fixture.snapshots();
    assert_eq!(snapshots.len(), 1);
    let snapshot = &snapshots[0];
    assert_eq!(snapshot.model, MODEL);
    assert_eq!(snapshot.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(snapshot.original_response, cached_json(&cached_chat()));
    assert_eq!(hit_field(&snapshot.caching_details), Some(true));
    assert_eq!(duration_field(&snapshot.caching_details), Some(75.0));
    assert_eq!(
        snapshot.litellm_params.get("acompletion"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        snapshot.litellm_params.get("custom_llm_provider"),
        Some(&Json::Text("openai".to_owned()))
    );
    assert_eq!(
        snapshot.litellm_params.get("preset_cache_key"),
        Some(&Json::Text(key.clone()))
    );
    let response = outcome.response.expect("a hit returns a response");
    let Some(Replayed::Response(ResponseKind::Chat, model)) = response.cached_result else {
        panic!("the cached chat response is converted");
    };
    assert!(model.cache_hit);
    assert_eq!(model.stamp, Some(Some(key)));
    let recorded = fixture.conversions.recorded();
    assert_eq!(recorded.kinds, [ResponseKind::Chat]);
    assert_eq!(
        recorded.metadata,
        [Metadata {
            model: MODEL.to_owned(),
            elapsed: Duration::from_secs(2),
        }]
    );
}

#[tokio::test]
async fn a_sync_hit_records_the_provider_prefixed_model_without_a_duration() {
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
        lookup(CachingSupportedCallType::Completion, request_kwargs),
        &mut logging,
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the sync lookup runs");
    let snapshots = fixture.snapshots();
    assert_eq!(snapshots.len(), 1);
    let snapshot = &snapshots[0];
    assert_eq!(snapshot.model, "openai/gpt-4o");
    assert_eq!(snapshot.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        snapshot.litellm_params.get("acompletion"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        snapshot.litellm_params.get("preset_cache_key"),
        Some(&Json::Null)
    );
    assert_eq!(hit_field(&snapshot.caching_details), Some(true));
    assert_eq!(
        details_fields(&snapshot.caching_details).get("cache_duration_ms"),
        Some(&Json::Null)
    );
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "sync"]);
    assert_eq!(
        logging
            .state
            .model_call_details
            .get("has_logged_sync_success"),
        Some(&Json::Bool(true))
    );
    let Some(Replayed::Response(ResponseKind::Chat, model)) = response.cached_result else {
        panic!("the cached chat response is converted");
    };
    assert!(model.cache_hit);
    assert_eq!(model.stamp, Some(Some(key)));
    let recorded = fixture.conversions.recorded();
    assert_eq!(
        recorded.metadata,
        [Metadata {
            model: MODEL.to_owned(),
            elapsed: Duration::from_secs(2),
        }]
    );
}

#[tokio::test]
async fn a_request_cache_key_reaches_the_context_and_the_stamp() {
    let fixture = Fixture::new();
    let mut request_kwargs = chat_kwargs();
    request_kwargs.insert("cache_key".to_owned(), text("request-key"));
    fixture.backend.seed("request-key", cached_chat());
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
        lookup(CachingSupportedCallType::Acompletion, request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the async lookup runs");
    worker.flush().await;
    let snapshots = fixture.snapshots();
    assert_eq!(
        snapshots[0].litellm_params.get("preset_cache_key"),
        Some(&Json::Text("request-key".to_owned()))
    );
    let response = outcome.response.expect("a hit returns a response");
    let Some(Replayed::Response(_, model)) = response.cached_result else {
        panic!("the cached chat response is converted");
    };
    assert_eq!(model.stamp, Some(Some("request-key".to_owned())));
}

#[test]
fn the_preset_cache_key_wins_over_the_request_key() {
    let mut handler = handler_named("acompletion", &["model", "messages"], chat_kwargs());
    handler.preset_cache_key = Some("preset-key".to_owned());
    handler
        .request_kwargs
        .insert("cache_key".to_owned(), text("request-key"));
    let key = hit_cache_key(&handler, None).expect("the cache key resolves");
    assert_eq!(key, Some("preset-key".to_owned()));
}

#[test]
fn the_request_cache_key_is_used_when_no_preset_exists() {
    let mut handler = handler_named("acompletion", &["model", "messages"], chat_kwargs());
    handler
        .request_kwargs
        .insert("cache_key".to_owned(), text("request-key"));
    let key = hit_cache_key(&handler, None).expect("the cache key resolves");
    assert_eq!(key, Some("request-key".to_owned()));
}

#[test]
fn the_cache_key_is_computed_from_the_request_kwargs() {
    let fixture = Fixture::new();
    let handler = handler_named("acompletion", &["model", "messages"], chat_kwargs());
    let source = fixture.source();
    let expected = computed_key(&fixture.cache, &chat_kwargs());
    let key = hit_cache_key(&handler, Some(&source)).expect("the cache key resolves");
    assert_eq!(key, Some(expected));
}

#[test]
fn the_cache_key_stays_none_without_a_cache_to_compute_from() {
    let handler = handler_named("acompletion", &["model", "messages"], chat_kwargs());
    let key = hit_cache_key(&handler, None).expect("the cache key resolves");
    assert_eq!(key, None);
}

#[test]
fn the_cache_duration_is_the_elapsed_milliseconds() {
    let timing = CacheTiming {
        started: Duration::from_millis(250),
        finished: Duration::from_millis(1_000),
    };
    assert_eq!(cache_duration(timing), 750.0);
}

#[test]
fn a_backwards_cache_timing_counts_no_time() {
    let timing = CacheTiming {
        started: Duration::from_millis(250),
        finished: Duration::from_millis(100),
    };
    assert_eq!(cache_duration(timing), 0.0);
}

#[test]
fn the_hit_kind_sets_the_async_and_embedding_flags() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution.context();
    let request = chat_kwargs();
    let record = hit_record(&resolution, MODEL, &request, &cached_chat(), Some(12.5))
        .expect("the hit record builds");
    assert_eq!(record.provider.model, MODEL);
    assert_eq!(record.provider.provider, "openai");
    let async_hit = record.context(MODEL, Some("preset-key"), HitKind::AsyncResponse);
    assert!(async_hit.is_async);
    assert!(!async_hit.is_embedding);
    assert_eq!(async_hit.cache_duration_ms, Some(12.5));
    assert_eq!(async_hit.custom_llm_provider, Some("openai"));
    assert_eq!(async_hit.preset_cache_key, Some("preset-key"));
    let embedding = record.context(MODEL, None, HitKind::AsyncEmbedding);
    assert!(embedding.is_async);
    assert!(embedding.is_embedding);
    let sync_hit = record.context(MODEL, None, HitKind::SyncResponse);
    assert!(!sync_hit.is_async);
    assert!(!sync_hit.is_embedding);
}

#[rstest::rstest]
#[case::custom_provider("custom_llm_provider")]
#[case::api_base("api_base")]
#[case::api_key("api_key")]
#[test]
fn a_provider_field_of_the_wrong_type_is_refused(#[case] field: &'static str) {
    let fixture = Fixture::new();
    let resolution = fixture.resolution.context();
    let request = super::support::kwargs(&[(field, KeyValue::Bool(true))]);
    let error = match resolve_provider(&resolution, MODEL, &request) {
        Ok(_provider) => panic!("the provider field is refused"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        HitError::FieldType {
            field: refused,
            ..
        } if refused == field
    ));
}

#[test]
fn a_null_provider_field_names_no_provider() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution.context();
    let request = super::support::kwargs(&[("custom_llm_provider", KeyValue::Null)]);
    let provider = resolve_provider(&resolution, MODEL, &request).expect("the provider resolves");
    assert_eq!(provider.provider, "openai");
    assert_eq!(provider.model, MODEL);
}

#[test]
fn the_async_hit_writes_the_parent_span_back_as_a_traceparent() {
    let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let mut request: KeyObject = chat_kwargs();
    request.insert(
        "metadata".to_owned(),
        object(&[("litellm_parent_otel_span", text(traceparent))]),
    );
    add_parent_otel_span(&mut request);
    assert_eq!(request.get("parent_otel_span"), Some(&text(traceparent)));
}

#[test]
fn an_unreadable_parent_span_is_written_back_as_null() {
    let mut request: KeyObject = chat_kwargs();
    add_parent_otel_span(&mut request);
    assert_eq!(request.get("parent_otel_span"), Some(&KeyValue::Null));
}
