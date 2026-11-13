use std::sync::Arc;

use http::Method;

use super::fixtures::{
    ConstantEmbedder, FakeQdrant, Reply, SIMILARITY, answer, body_of, cache_with, field, json,
    kwargs, metadata, search_hit, search_result_null, similarity_of,
};
use crate::arguments::json::{Json, Object};
use crate::qdrant_semantic_cache::{QdrantCacheKwargs, QdrantSemanticCache};

const CACHED: &str = r#""{\"choices\":[]}""#;

fn search_cache(reply: Reply) -> (QdrantSemanticCache, Arc<FakeQdrant>) {
    let transport = Arc::new(FakeQdrant::new(0.9_f64, vec![reply]));
    let cache = cache_with(Arc::clone(&transport), Arc::new(ConstantEmbedder::new()));
    (cache, transport)
}

async fn lookup(cache: &QdrantSemanticCache, key: &str) -> (Option<Json>, Object) {
    let mut found = metadata();
    let value = cache
        .async_get_cache(key, &kwargs(), Some(&mut found))
        .await
        .expect("the cache lookup");
    (value, found)
}

#[tokio::test]
async fn search_filter_carries_the_cache_key() {
    let (cache, transport) = search_cache(search_hit(0.93, Some("scope-1"), CACHED));
    let _value = lookup(&cache, "scope-1").await;
    let request = &transport.requests()[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.path, "/collections/semantic-cache/points/search");
    assert!(request.query.is_empty());
    let body = body_of(request);
    assert_eq!(field(&body, "vector"), &json("[0.25,0.5,0.75]"));
    assert_eq!(field(&body, "limit"), &json("1"));
    assert_eq!(field(&body, "with_payload"), &json("true"));
    assert_eq!(
        field(field(&body, "params"), "quantization"),
        &json(r#"{"ignore":false,"rescore":true,"oversampling":3.0}"#)
    );
    assert_eq!(
        field(&body, "filter"),
        &json(r#"{"must":[{"key":"litellm_cache_key","match":{"value":"scope-1"}}]}"#)
    );
}

#[tokio::test]
async fn payload_matching_key_returns_the_cached_response() {
    let (cache, _transport) = search_cache(search_hit(0.93, Some("scope-1"), CACHED));
    let (value, found) = lookup(&cache, "scope-1").await;
    assert_eq!(value, Some(json(r#"{"choices":[]}"#)));
    assert_eq!(similarity_of(&found), Some(0.93_f64));
    assert_eq!(found.get("request-id"), Some(&json(r#""abc""#)));
}

#[tokio::test]
async fn payload_missing_cache_key_is_a_miss() {
    let (cache, _transport) = search_cache(search_hit(0.99, None, CACHED));
    let (value, found) = lookup(&cache, "scope-1").await;
    assert_eq!(value, None);
    assert_eq!(similarity_of(&found), Some(0.0_f64));
}

#[tokio::test]
async fn hit_scope_with_another_key_is_a_miss() {
    let (cache, _transport) = search_cache(search_hit(0.99, Some("scope-2"), CACHED));
    let (value, found) = lookup(&cache, "scope-1").await;
    assert_eq!(value, None);
    assert_eq!(similarity_of(&found), Some(0.0_f64));
}

#[tokio::test]
async fn similarity_threshold_above_returns_the_response() {
    let (cache, _transport) = search_cache(search_hit(0.93, Some("scope-1"), CACHED));
    let (value, _found) = lookup(&cache, "scope-1").await;
    assert!(value.is_some());
}

#[tokio::test]
async fn similarity_threshold_equality_returns_the_response() {
    let (cache, _transport) = search_cache(search_hit(0.8, Some("scope-1"), CACHED));
    let (value, _found) = lookup(&cache, "scope-1").await;
    assert!(value.is_some());
}

#[tokio::test]
async fn similarity_below_threshold_misses_the_response() {
    let (cache, _transport) = search_cache(search_hit(0.42, Some("scope-1"), CACHED));
    let (value, found) = lookup(&cache, "scope-1").await;
    assert_eq!(value, None);
    assert_eq!(similarity_of(&found), Some(0.42_f64));
}

#[tokio::test]
async fn similarity_metadata_records_the_actual_score() {
    let (cache, _transport) = search_cache(search_hit(0.42, Some("scope-1"), CACHED));
    let (_value, found) = lookup(&cache, "scope-1").await;
    assert_eq!(similarity_of(&found), Some(0.42_f64));
    assert!(found.contains_key(SIMILARITY));
}

#[tokio::test]
async fn similarity_metadata_records_zero_without_a_hit() {
    let (cache, _transport) = search_cache(search_result_null());
    let (value, found) = lookup(&cache, "scope-1").await;
    assert_eq!(value, None);
    assert_eq!(similarity_of(&found), Some(0.0_f64));
}

#[tokio::test]
async fn response_result_null_is_a_miss() {
    let (cache, _transport) = search_cache(search_result_null());
    let (value, _found) = lookup(&cache, "scope-1").await;
    assert_eq!(value, None);
}

#[tokio::test]
async fn search_result_without_hits_is_a_miss() {
    let (cache, _transport) = search_cache(answer(200, r#"{"result":[]}"#));
    let (value, _found) = lookup(&cache, "scope-1").await;
    assert_eq!(value, None);
}

#[tokio::test]
async fn search_response_rejects_invalid_json() {
    let (cache, _transport) = search_cache(answer(200, "not json"));
    let mut found = metadata();
    let error = cache
        .async_get_cache("scope-1", &kwargs(), Some(&mut found))
        .await
        .expect_err("an unparsable search response must fail");
    assert!(error.to_string().contains("points search"));
    assert_eq!(similarity_of(&found), None);
}

#[tokio::test]
async fn response_parse_error_names_the_missing_result() {
    let (cache, _transport) = search_cache(answer(200, "{}"));
    let mut found = metadata();
    let error = cache
        .async_get_cache("scope-1", &kwargs(), Some(&mut found))
        .await
        .expect_err("a search response without result must fail");
    assert!(error.to_string().contains("points search"));
    assert!(error.to_string().contains("result"));
}

#[tokio::test]
async fn search_error_response_is_an_error_not_a_miss() {
    let body = r#"{"status":{"error":"Not found: Collection `semantic-cache` does not exist!"}}"#;
    let (cache, _transport) = search_cache(answer(404, body));
    let mut found = metadata();
    let error = cache
        .async_get_cache("scope-1", &kwargs(), Some(&mut found))
        .await
        .expect_err("a Qdrant error reply must fail the read");
    assert!(error.to_string().contains("points search"));
    assert!(error.to_string().contains("result"));
    assert_eq!(similarity_of(&found), None);
}

#[tokio::test]
async fn search_hit_without_text_errors() {
    let body =
        r#"{"result":[{"score":0.99,"payload":{"litellm_cache_key":"scope-1","response":"{}"}}]}"#;
    let (cache, _transport) = search_cache(answer(200, body));
    let mut found = metadata();
    let error = cache
        .async_get_cache("scope-1", &kwargs(), Some(&mut found))
        .await
        .expect_err("a hit without text must fail");
    assert!(error.to_string().contains("text"));
}

#[tokio::test]
async fn search_hit_without_a_response_errors_when_it_hits() {
    let body = r#"{"result":[{"score":0.99,"payload":{"litellm_cache_key":"scope-1","text":"what is cached"}}]}"#;
    let (cache, _transport) = search_cache(answer(200, body));
    let mut found = metadata();
    let error = cache
        .async_get_cache("scope-1", &kwargs(), Some(&mut found))
        .await
        .expect_err("a hit without a response must fail");
    assert!(error.to_string().contains("response"));
}

#[tokio::test]
async fn search_without_messages_returns_no_hit() {
    let (cache, transport) = search_cache(search_result_null());
    let mut found = metadata();
    let empty = QdrantCacheKwargs {
        messages: None,
        metadata: None,
    };
    let value = cache
        .async_get_cache("scope-1", &empty, Some(&mut found))
        .await
        .expect("a lookup without messages");
    assert_eq!(value, None);
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn search_embedding_failure_propagates() {
    let transport = Arc::new(FakeQdrant::new(0.9_f64, Vec::new()));
    let cache = cache_with(
        Arc::clone(&transport),
        Arc::new(ConstantEmbedder::failing()),
    );
    let mut found = metadata();
    let error = cache
        .async_get_cache("scope-1", &kwargs(), Some(&mut found))
        .await
        .expect_err("an embedding failure must fail the lookup");
    assert_eq!(error.to_string(), "the embedding provider failed");
    assert!(transport.requests().is_empty());
}
