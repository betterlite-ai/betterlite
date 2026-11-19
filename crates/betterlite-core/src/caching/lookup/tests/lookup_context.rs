use std::time::Duration;

use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::replay::{Replayed, ResponseKind};
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_models::decisions::Json;

use super::support::{
    Fixture, MODEL, Metadata, Model, SilentConsole, SilentLogger, Stamp, Tokens,
    async_cache_lookup, cached_chat, cached_json, chat_kwargs, computed_key, details_fields,
    duration_field, handler_named, hit_field, kwargs, lookup, object, perf_clock,
    sync_cache_lookup, text,
};
use crate::caching::lookup::error::HitError;
use crate::caching::lookup::hit::{
    HitKind, HitLogging, hit_cache_key, hit_record, resolve_provider,
};

fn converted_chat(response: Option<Replayed<Model>>) -> Model {
    let Some(Replayed::Response(ResponseKind::Chat, model)) = response else {
        panic!("the cached chat response is converted");
    };
    model
}

fn assert_chat_conversion(fixture: &Fixture) {
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

fn assert_async_chat_snapshot(fixture: &Fixture, preset: &str) {
    let snapshots = fixture.snapshots();
    assert_eq!(snapshots.len(), 1);
    let snapshot = &snapshots[0];
    assert_eq!(snapshot.model, MODEL);
    assert_eq!(snapshot.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(snapshot.original_response, cached_json(&cached_chat()));
    assert_eq!(hit_field(&snapshot.caching_details), Some(true));
    assert_eq!(duration_field(&snapshot.caching_details), Some(75.0_f64));
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
        Some(&Json::Text(preset.to_owned()))
    );
}

fn assert_sync_chat_snapshot(fixture: &Fixture, logging: &HitLogging) {
    let snapshots = fixture.snapshots();
    assert_eq!(snapshots.len(), 1);
    let snapshot = &snapshots[0];
    assert_eq!(snapshot.model, "openai/gpt-4o");
    assert_eq!(snapshot.custom_llm_provider.as_deref(), Some("openai"));
    // the sync path updates the environment after converting, so the converted result is the one recorded
    assert_eq!(snapshot.original_response, Json::Bool(true));
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
}

#[tokio::test]
async fn an_async_hit_records_the_resolved_model_and_cache_duration() {
    let fixture = Fixture::new();
    // the request spells the provider out, so the hit must record the model resolution strips it to
    let mut request_kwargs = chat_kwargs();
    request_kwargs.insert("model".to_owned(), text("openai/gpt-4o"));
    let key = computed_key(&fixture.cache, &request_kwargs);
    fixture.backend.seed(&key, cached_chat());
    let mut handler = handler_named(
        "acompletion",
        &["model", "messages"],
        request_kwargs.clone(),
    );
    let mut request = lookup(CachingSupportedCallType::Acompletion, request_kwargs);
    request.model = "openai/gpt-4o".to_owned();
    let tokens = Tokens::default();
    let (outcome, worker) = async_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        &tokens,
        perf_clock(&[Duration::from_millis(100), Duration::from_millis(175)]),
    )
    .await;
    worker.flush().await;
    assert_async_chat_snapshot(&fixture, &key);
    let model = converted_chat(
        outcome
            .response
            .expect("a hit returns a response")
            .cached_result,
    );
    assert!(model.cache_hit);
    assert_eq!(model.stamp, Stamp::Key(key));
    assert_chat_conversion(&fixture);
}

#[tokio::test]
async fn a_sync_hit_records_the_provider_prefixed_model_without_a_duration() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
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
    assert_sync_chat_snapshot(&fixture, &logging);
    let model = converted_chat(response.cached_result);
    assert!(model.cache_hit);
    assert_eq!(model.stamp, Stamp::Key(key));
    assert_chat_conversion(&fixture);
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
    worker.flush().await;
    assert_eq!(
        fixture.snapshots()[0]
            .litellm_params
            .get("preset_cache_key"),
        Some(&Json::Text("request-key".to_owned()))
    );
    let model = converted_chat(
        outcome
            .response
            .expect("a hit returns a response")
            .cached_result,
    );
    assert_eq!(model.stamp, Stamp::Key("request-key".to_owned()));
}

#[test]
fn the_preset_cache_key_wins_over_the_request_key() {
    let mut handler = handler_named("acompletion", &["model", "messages"], chat_kwargs());
    handler.preset_cache_key = Some("preset-key".to_owned());
    handler
        .request_kwargs
        .insert("cache_key".to_owned(), text("request-key"));
    let key = hit_cache_key::<SilentLogger, SilentConsole>(&handler, None)
        .expect("the cache key resolves");
    assert_eq!(key, Some("preset-key".to_owned()));
}

#[test]
fn the_request_cache_key_is_used_when_no_preset_exists() {
    let mut handler = handler_named("acompletion", &["model", "messages"], chat_kwargs());
    handler
        .request_kwargs
        .insert("cache_key".to_owned(), text("request-key"));
    let key = hit_cache_key::<SilentLogger, SilentConsole>(&handler, None)
        .expect("the cache key resolves");
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
    let key = hit_cache_key::<SilentLogger, SilentConsole>(&handler, None)
        .expect("the cache key resolves");
    assert_eq!(key, None);
}

#[tokio::test]
async fn a_backwards_cache_timing_counts_no_time() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
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
        perf_clock(&[Duration::from_millis(250), Duration::from_millis(100)]),
    )
    .await;
    worker.flush().await;
    assert!(outcome.response.is_some());
    assert_eq!(
        duration_field(&fixture.snapshots()[0].caching_details),
        Some(0.0_f64)
    );
}

#[test]
fn the_hit_kind_sets_the_async_and_embedding_flags() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution.context();
    let request = chat_kwargs();
    let record = hit_record(&resolution, MODEL, &request, &cached_chat(), Some(12.5_f64))
        .expect("the hit record builds");
    assert_eq!(record.provider.model, MODEL);
    assert_eq!(record.provider.provider, "openai");
    let cached = &record.cached_result;
    let async_hit = record.context(MODEL, Some("preset-key"), HitKind::AsyncResponse, cached);
    assert!(async_hit.is_async);
    assert!(!async_hit.is_embedding);
    assert_eq!(async_hit.cache_duration_ms, Some(12.5_f64));
    assert_eq!(async_hit.custom_llm_provider, Some("openai"));
    assert_eq!(async_hit.preset_cache_key, Some("preset-key"));
    assert_eq!(async_hit.cached_result, cached);
    let embedding = record.context(MODEL, None, HitKind::AsyncEmbedding, cached);
    assert!(embedding.is_async);
    assert!(embedding.is_embedding);
    let sync_hit = record.context(MODEL, None, HitKind::SyncResponse, cached);
    assert!(!sync_hit.is_async);
    assert!(!sync_hit.is_embedding);
}

#[rstest::rstest]
#[case::custom_provider("custom_llm_provider")]
#[case::api_base("api_base")]
#[case::api_key("api_key")]
fn a_provider_field_of_the_wrong_type_is_refused(#[case] field: &'static str) {
    let fixture = Fixture::new();
    let resolution = fixture.resolution.context();
    let request = kwargs(&[(field, KeyValue::Bool(true))]);
    let Err(error) = resolve_provider(&resolution, MODEL, &request) else {
        panic!("the provider field is refused");
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
    let request = kwargs(&[("custom_llm_provider", KeyValue::Null)]);
    let provider = resolve_provider(&resolution, MODEL, &request).expect("the provider resolves");
    assert_eq!(provider.provider, "openai");
    assert_eq!(provider.model, MODEL);
}

#[tokio::test]
async fn the_async_lookup_writes_the_parent_span_into_the_request_kwargs() {
    let fixture = Fixture::new();
    let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0c9d02b7-01";
    let mut request_kwargs = chat_kwargs();
    request_kwargs.insert(
        "metadata".to_owned(),
        object(&[("litellm_parent_otel_span", text(traceparent))]),
    );
    let mut handler = handler_named(
        "acompletion",
        &["model", "messages"],
        request_kwargs.clone(),
    );
    let request = lookup(CachingSupportedCallType::Acompletion, request_kwargs);
    let tokens = Tokens::default();
    let (_outcome, _worker) = async_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    )
    .await;
    assert_eq!(
        handler.request_kwargs.get("parent_otel_span"),
        Some(&text(traceparent))
    );
}

#[tokio::test]
async fn a_lookup_without_a_parent_span_writes_null_into_the_request_kwargs() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    let mut handler = handler_named(
        "acompletion",
        &["model", "messages"],
        request_kwargs.clone(),
    );
    let request = lookup(CachingSupportedCallType::Acompletion, request_kwargs);
    let tokens = Tokens::default();
    let (_outcome, _worker) = async_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    )
    .await;
    assert_eq!(
        handler.request_kwargs.get("parent_otel_span"),
        Some(&KeyValue::Null)
    );
}
