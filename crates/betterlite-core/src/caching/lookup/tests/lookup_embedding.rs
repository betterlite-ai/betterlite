use std::time::Duration;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::embedding_merge::response::{CachedEmbeddingResponse, Embedding};
use betterlite_cache::embedding_merge::usage::Usage;
use betterlite_cache::handler::state::CachingHandler;
use betterlite_cache::types::embedding::{EMBEDDING_CACHE_FORMAT_VERSION, EmbeddingPayload};
use betterlite_models::decisions::{HiddenParams, Json};
use serde_json::Number;

use super::support::{
    EMBEDDING_MODEL, Fixture, ReadKind, Tokens, details_fields, duration_field, embedding_key,
    embedding_kwargs, embedding_lookup, handler_named, hit_field, perf_clock, services,
    stored_embedding, text, worker,
};
use crate::caching::lookup::async_lookup::async_get_cache;

fn vector(values: &[f64]) -> EmbeddingPayload {
    EmbeddingPayload::Vector(
        values
            .iter()
            .map(|value| Number::from_f64(*value).expect("the fixture value is finite"))
            .collect(),
    )
}

fn cached_hit_response() -> HiddenParams {
    HiddenParams::from([("cache_hit".to_owned(), Json::Bool(true))])
}

fn embedding_handler(request: &KeyObject) -> CachingHandler {
    handler_named("aembedding", &["model", "input"], request.clone())
}

#[tokio::test]
async fn a_partial_embedding_hit_merges_the_cached_positions() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored_embedding(&[0.5, 0.25], Some(2), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    let mut handler = embedding_handler(&request_kwargs);
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
        &embedding_lookup(request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the embedding lookup runs");
    assert_eq!(fixture.backend.reads(), [ReadKind::Async, ReadKind::Async]);
    let response = outcome
        .response
        .expect("the partial hit returns a response");
    assert!(response.cached_result.is_none());
    assert!(!response.all_embeddings_cached);
    assert_eq!(
        response.final_embedding_response,
        Some(CachedEmbeddingResponse {
            model: Some(EMBEDDING_MODEL.to_owned()),
            data: vec![
                Some(Embedding {
                    embedding: vector(&[0.5, 0.25]),
                    index: 0,
                }),
                None,
            ],
            usage: Some(Usage {
                prompt_tokens: 2,
                completion_tokens: 0,
                total_tokens: 2,
                prompt_tokens_details: None,
            }),
            hidden_params: cached_hit_response(),
            response_headers: None,
        })
    );
    assert_eq!(response.embedding_uncached_input, Some(vec![text("beta")]));
    assert_eq!(
        handler.request_kwargs.get("input"),
        Some(&KeyValue::Array(vec![text("beta")]))
    );
    assert!(outcome.logging.is_some());
    assert_eq!(worker.stats().accepted, 0);
    assert!(fixture.calls.events().is_empty());
    assert!(tokens.counted().is_empty());
}

#[tokio::test]
async fn a_full_embedding_hit_logs_the_merged_usage_once() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored_embedding(&[0.5], Some(2), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "beta"),
        stored_embedding(&[0.25], None, EMBEDDING_CACHE_FORMAT_VERSION),
    );
    let mut handler = embedding_handler(&request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let mut tokens = Tokens::default();
    tokens.count = 7;
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
        &embedding_lookup(request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the embedding lookup runs");
    let report = worker.flush().await;
    assert_eq!(report.completed, 1);
    assert!(outcome.logging.is_none());
    assert_eq!(fixture.calls.events(), ["build-with-cache-hit", "async"]);
    assert_eq!(tokens.counted(), ["beta"]);
    let response = outcome.response.expect("the full hit returns a response");
    assert!(response.all_embeddings_cached);
    assert_eq!(response.embedding_uncached_input, Some(Vec::new()));
    let final_response = response
        .final_embedding_response
        .expect("the cached positions are merged");
    assert_eq!(
        final_response.usage,
        Some(Usage {
            prompt_tokens: 9,
            completion_tokens: 0,
            total_tokens: 9,
            prompt_tokens_details: None,
        })
    );
    let snapshots = fixture.snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].model, EMBEDDING_MODEL);
    assert_eq!(snapshots[0].custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(hit_field(&snapshots[0].caching_details), Some(true));
    assert_eq!(
        details_fields(&snapshots[0].caching_details).get("cache_duration_ms"),
        Some(&Json::Null)
    );
    assert_eq!(duration_field(&snapshots[0].caching_details), None);
    assert_eq!(snapshots[0].original_response, snapshots[0].logged_response);
    let Json::Object(logged) = &snapshots[0].logged_response else {
        panic!("the embedding response is logged");
    };
    assert_eq!(
        logged.get("model"),
        Some(&Json::Text(EMBEDDING_MODEL.to_owned()))
    );
    let Some(Json::Object(usage)) = logged.get("usage") else {
        panic!("the merged usage is logged");
    };
    assert_eq!(
        usage.get("prompt_tokens"),
        Some(&Json::Number(Number::from(9)))
    );
    assert_eq!(
        usage.get("total_tokens"),
        Some(&Json::Number(Number::from(9)))
    );
    let Some(Json::List(data)) = logged.get("data") else {
        panic!("the merged positions are logged");
    };
    assert_eq!(data.len(), 2);
}

#[tokio::test]
async fn every_cached_prompt_token_count_skips_the_counter() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored_embedding(&[0.5], Some(2), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "beta"),
        stored_embedding(&[0.25], Some(3), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    let mut handler = embedding_handler(&request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let mut tokens = Tokens::default();
    tokens.count = 99;
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
        &embedding_lookup(request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the embedding lookup runs");
    worker.flush().await;
    assert!(tokens.counted().is_empty());
    let response = outcome.response.expect("the full hit returns a response");
    let final_response = response
        .final_embedding_response
        .expect("the cached positions are merged");
    assert_eq!(
        final_response
            .usage
            .expect("the merge sums the usage")
            .prompt_tokens,
        5
    );
}

#[tokio::test]
async fn an_old_format_entry_is_treated_as_uncached() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored_embedding(&[0.5], Some(2), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "beta"),
        stored_embedding(&[0.25], Some(3), EMBEDDING_CACHE_FORMAT_VERSION - 1),
    );
    let mut handler = embedding_handler(&request_kwargs);
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
        &embedding_lookup(request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the embedding lookup runs");
    let response = outcome
        .response
        .expect("the partial hit returns a response");
    assert!(!response.all_embeddings_cached);
    assert_eq!(response.embedding_uncached_input, Some(vec![text("beta")]));
    let final_response = response
        .final_embedding_response
        .expect("the cached positions are merged");
    assert!(final_response.data[1].is_none());
    assert_eq!(
        final_response
            .usage
            .expect("the merge sums the usage")
            .prompt_tokens,
        2
    );
    assert_eq!(fixture.backend.reads(), [ReadKind::Async, ReadKind::Async]);
}
