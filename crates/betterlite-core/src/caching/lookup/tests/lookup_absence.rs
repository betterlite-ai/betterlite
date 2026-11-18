use std::time::Duration;

use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::replay::Replayed;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_cache::types::embedding::EMBEDDING_CACHE_FORMAT_VERSION;
use betterlite_context::local_ttl_cache::CacheValue;

use super::support::{
    Fixture, ReadKind, Tokens, cached_chat, cached_detail, cached_json, chat_kwargs, computed_key,
    embedding_key, embedding_kwargs, embedding_lookup, handler_named, hit_field, lookup,
    perf_clock, s3_services, services, stored_embedding, worker,
};
use crate::caching::hit_callbacks::context::CACHING_DETAILS;
use crate::caching::lookup::async_lookup::async_get_cache;
use crate::caching::lookup::sync_lookup::sync_get_cache;

#[tokio::test]
async fn a_disabled_async_lookup_returns_none() {
    let fixture = Fixture::new();
    let mut request_kwargs = chat_kwargs();
    request_kwargs.insert("caching".to_owned(), KeyValue::Bool(false));
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        cached_chat(),
    );
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
    .expect("the eligibility gate runs");
    assert!(outcome.response.is_none());
    assert!(outcome.logging.is_some());
    assert!(fixture.backend.reads().is_empty());
}

#[tokio::test]
async fn an_async_lookup_without_a_cache_returns_none() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    let mut handler = handler_named(
        "acompletion",
        &["model", "messages"],
        request_kwargs.clone(),
    );
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        None,
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
    .expect("the eligibility gate runs");
    assert!(outcome.response.is_none());
    assert!(outcome.logging.is_some());
    assert!(fixture.backend.reads().is_empty());
}

#[tokio::test]
async fn an_async_miss_returns_an_empty_response() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
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
    assert_eq!(fixture.backend.reads(), [ReadKind::Async]);
    assert!(outcome.logging.is_some());
    let response = outcome.response.expect("a miss returns a response");
    assert!(response.cached_result.is_none());
    assert!(response.final_embedding_response.is_none());
    assert!(!response.all_embeddings_cached);
    assert!(response.embedding_uncached_input.is_none());
}

#[tokio::test]
async fn a_sync_miss_reads_synchronously() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
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
    let response = sync_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Completion, request_kwargs),
        &mut logging,
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the sync lookup runs");
    assert_eq!(fixture.backend.reads(), [ReadKind::Sync]);
    assert!(response.cached_result.is_none());
    assert!(
        !logging
            .state
            .model_call_details
            .contains_key(CACHING_DETAILS)
    );
}

#[tokio::test]
async fn a_sync_detail_hit_is_returned_unconverted() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        cached_detail(),
    );
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
    let Some(Replayed::Unchanged(value)) = response.cached_result else {
        panic!("the stored error is returned as it is");
    };
    assert_eq!(value, cached_detail());
    assert!(fixture.conversions.recorded().kinds.is_empty());
    assert!(fixture.calls.events().is_empty());
    assert!(
        !logging
            .state
            .model_call_details
            .contains_key(CACHING_DETAILS)
    );
}

#[tokio::test]
async fn an_async_list_hit_is_returned_as_stored() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    let stored = CacheValue::Array(vec![CacheValue::Integer(1)]);
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        stored.clone(),
    );
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
    assert!(outcome.logging.is_some());
    assert_eq!(worker.stats().accepted, 0);
    assert!(fixture.calls.events().is_empty());
    assert!(fixture.conversions.recorded().kinds.is_empty());
    let response = outcome.response.expect("a hit returns a response");
    let Some(Replayed::Unchanged(value)) = response.cached_result else {
        panic!("a stored list is returned as it is");
    };
    assert_eq!(value, stored);
}

#[tokio::test]
async fn a_sync_list_hit_is_logged_as_stored() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    let stored = CacheValue::Array(vec![CacheValue::Integer(1)]);
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        stored.clone(),
    );
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
    let Some(Replayed::Unchanged(value)) = response.cached_result else {
        panic!("a stored list is returned as it is");
    };
    assert_eq!(value, stored);
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "sync"]);
    let snapshots = fixture.snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].logged_response, cached_json(&stored));
    assert_eq!(snapshots[0].original_response, cached_json(&stored));
    assert_eq!(hit_field(&snapshots[0].caching_details), Some(true));
}

#[tokio::test]
async fn an_old_format_embedding_entry_is_a_miss() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha"]);
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored_embedding(&[0.5], Some(2), EMBEDDING_CACHE_FORMAT_VERSION - 1),
    );
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
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
        embedding_lookup(request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the embedding lookup runs");
    assert_eq!(fixture.backend.reads(), [ReadKind::Async]);
    assert!(outcome.logging.is_some());
    let response = outcome.response.expect("a miss returns a response");
    assert!(response.cached_result.is_none());
    assert!(response.final_embedding_response.is_none());
    assert!(!response.all_embeddings_cached);
    assert!(response.embedding_uncached_input.is_none());
}

#[tokio::test]
async fn an_s3_partial_embedding_hit_returns_the_stored_entries() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored_embedding(&[0.5], Some(2), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    let mut handler = handler_named("aembedding", &["model", "input"], request_kwargs.clone());
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = s3_services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let outcome = async_get_cache(
        &mut handler,
        embedding_lookup(request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the embedding lookup runs");
    assert!(outcome.logging.is_some());
    assert_eq!(worker.stats().accepted, 0);
    assert!(fixture.calls.events().is_empty());
    assert!(tokens.counted().is_empty());
    let response = outcome.response.expect("a hit returns a response");
    assert!(response.final_embedding_response.is_none());
    assert!(!response.all_embeddings_cached);
    assert!(response.embedding_uncached_input.is_none());
    let Some(Replayed::Unchanged(CacheValue::Array(items))) = response.cached_result else {
        panic!("the stored entries are returned as they are");
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(&items[0], CacheValue::Object(fields) if fields.contains_key("embedding")));
    assert_eq!(items[1], CacheValue::Null);
}
