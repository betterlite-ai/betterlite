use std::time::Duration;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::embedding_merge::response::{CachedEmbeddingResponse, Embedding};
use betterlite_cache::embedding_merge::usage::Usage;
use betterlite_cache::handler::state::CachingHandler;
use betterlite_cache::types::embedding::{EMBEDDING_CACHE_FORMAT_VERSION, EmbeddingPayload};
use betterlite_models::decisions::{HiddenParams, Json};
use serde_json::Number;

use super::support::{
    EMBEDDING_MODEL, Fixture, ReadKind, Tokens, async_cache_lookup, details_fields, duration_field,
    embedding_key, embedding_kwargs, embedding_lookup, handler_named, hit_field, perf_clock,
    stored_embedding, text,
};

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

const fn usage_counts(prompt: i64, completion: i64, total: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        prompt_tokens_details: None,
    }
}

fn partial_embedding_response() -> CachedEmbeddingResponse {
    CachedEmbeddingResponse {
        model: Some(EMBEDDING_MODEL.to_owned()),
        data: vec![
            Some(Embedding {
                embedding: vector(&[0.5_f64, 0.25_f64]),
                index: 0,
            }),
            None,
        ],
        usage: Some(usage_counts(2, 0, 2)),
        hidden_params: cached_hit_response(),
        response_headers: None,
    }
}

fn merged_usage(response: &CachedEmbeddingResponse) -> &Usage {
    response.usage.as_ref().expect("the merge sums the usage")
}

fn assert_embedding_logged(fixture: &Fixture) {
    let snapshots = fixture.snapshots();
    assert_eq!(snapshots.len(), 1);
    let snapshot = &snapshots[0];
    assert_eq!(snapshot.model, EMBEDDING_MODEL);
    assert_eq!(snapshot.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(hit_field(&snapshot.caching_details), Some(true));
    assert_eq!(
        details_fields(&snapshot.caching_details).get("cache_duration_ms"),
        Some(&Json::Null)
    );
    assert_eq!(duration_field(&snapshot.caching_details), None);
    assert_eq!(snapshot.original_response, snapshot.logged_response);
    let Json::Object(logged) = &snapshot.logged_response else {
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
        Some(&Json::Number(Number::from(9_i32)))
    );
    assert_eq!(
        usage.get("total_tokens"),
        Some(&Json::Number(Number::from(9_i32)))
    );
    let Some(Json::List(data)) = logged.get("data") else {
        panic!("the merged positions are logged");
    };
    assert_eq!(data.len(), 2);
    let Some(Json::Object(first)) = data.first() else {
        panic!("the cached position is logged as an embedding");
    };
    let Some(Json::List(values)) = first.get("embedding") else {
        panic!("the cached vector is logged");
    };
    let Some(Json::Number(value)) = values.first() else {
        panic!("the cached vector holds a number");
    };
    assert_eq!(value.as_f64(), Some(0.5_f64));
    let Some(Json::Number(index)) = first.get("index") else {
        panic!("the cached position keeps its index");
    };
    assert_eq!(index.as_u64(), Some(0));
    assert_eq!(
        first.get("object"),
        Some(&Json::Text("embedding".to_owned()))
    );
}

#[tokio::test]
async fn a_partial_embedding_hit_merges_the_cached_positions() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    let stored = stored_embedding(
        &[0.5_f64, 0.25_f64],
        Some(2),
        EMBEDDING_CACHE_FORMAT_VERSION,
    );
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored,
    );
    let mut handler = embedding_handler(&request_kwargs);
    let request = embedding_lookup(request_kwargs);
    let tokens = Tokens::default();
    let (outcome, worker) = async_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    )
    .await;
    assert_eq!(fixture.backend.reads(), [ReadKind::Async, ReadKind::Async]);
    let response = outcome.response.expect("a hit returns a response");
    assert!(response.cached_result.is_none());
    assert!(!response.all_embeddings_cached);
    assert_eq!(
        response.final_embedding_response,
        Some(partial_embedding_response())
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
        stored_embedding(&[0.5_f64], Some(2), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "beta"),
        stored_embedding(&[0.25_f64], None, EMBEDDING_CACHE_FORMAT_VERSION),
    );
    let mut handler = embedding_handler(&request_kwargs);
    let request = embedding_lookup(request_kwargs);
    let tokens = Tokens::counting(7);
    let (outcome, worker) = async_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    )
    .await;
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
    assert_eq!(merged_usage(&final_response), &usage_counts(9, 0, 9));
    assert_embedding_logged(&fixture);
}

#[tokio::test]
async fn every_cached_prompt_token_count_skips_the_counter() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored_embedding(&[0.5_f64], Some(2), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "beta"),
        stored_embedding(&[0.25_f64], Some(3), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    let mut handler = embedding_handler(&request_kwargs);
    let request = embedding_lookup(request_kwargs);
    let tokens = Tokens::counting(99);
    let (outcome, _worker) = async_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    )
    .await;
    assert!(tokens.counted().is_empty());
    let response = outcome.response.expect("the full hit returns a response");
    let final_response = response
        .final_embedding_response
        .expect("the cached positions are merged");
    assert_eq!(merged_usage(&final_response).prompt_tokens, 5);
}

#[tokio::test]
async fn an_old_format_entry_is_treated_as_uncached() {
    let fixture = Fixture::new();
    let request_kwargs = embedding_kwargs(&["alpha", "beta"]);
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "alpha"),
        stored_embedding(&[0.5_f64], Some(2), EMBEDDING_CACHE_FORMAT_VERSION),
    );
    fixture.backend.seed(
        &embedding_key(&fixture.cache, &request_kwargs, "beta"),
        stored_embedding(&[0.25_f64], Some(3), EMBEDDING_CACHE_FORMAT_VERSION - 1),
    );
    let mut handler = embedding_handler(&request_kwargs);
    let request = embedding_lookup(request_kwargs);
    let tokens = Tokens::default();
    let (outcome, _worker) = async_cache_lookup(
        &fixture,
        &mut handler,
        &request,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    )
    .await;
    let response = outcome
        .response
        .expect("the partial hit returns a response");
    assert!(!response.all_embeddings_cached);
    assert_eq!(response.embedding_uncached_input, Some(vec![text("beta")]));
    let final_response = response
        .final_embedding_response
        .expect("the cached positions are merged");
    assert!(final_response.data[1].is_none());
    assert_eq!(merged_usage(&final_response).prompt_tokens, 2);
    assert_eq!(fixture.backend.reads(), [ReadKind::Async, ReadKind::Async]);
}
