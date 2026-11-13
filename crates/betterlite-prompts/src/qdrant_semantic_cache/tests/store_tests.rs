use std::sync::Arc;

use http::Method;
use tokio::runtime::{Builder, Handle};
use uuid::Uuid;

use super::fixtures::{
    ConstantEmbedder, FIXED_ID, FakeQdrant, Reply, answer, body_of, cache_from, cache_with, field,
    json, kwargs, metadata, similarity_of, user_message,
};
use crate::arguments::json::Json;
use crate::qdrant_semantic_cache::error::QdrantSemanticError;
use crate::qdrant_semantic_cache::ports::{QdrantRequest, QdrantTransport};
use crate::qdrant_semantic_cache::{QdrantCacheKwargs, QdrantSemanticCache};

const VALUE: &str = r#"{"choices":[{"message":{"content":"hi"}}]}"#;

fn memory_cache(score: f64) -> (Arc<FakeQdrant>, QdrantSemanticCache) {
    let transport = Arc::new(FakeQdrant::new(score, Vec::new()));
    let concrete = Arc::clone(&transport);
    let backend: Arc<dyn QdrantTransport> = concrete;
    let cache = cache_from(backend, Arc::new(ConstantEmbedder::new()));
    (transport, cache)
}

fn stored_key(request: &QdrantRequest) -> String {
    let body = body_of(request);
    let Json::List(points) = field(&body, "points") else {
        panic!("a points request carries a list");
    };
    let payload = field(&points[0], "payload");
    field(payload, "litellm_cache_key")
        .as_str()
        .expect("a stored key")
        .to_owned()
}

#[test]
fn qdrant_semantic_cache_roundtrip() {
    let (transport, cache) = memory_cache(0.9);
    let runtime = Builder::new_current_thread()
        .build()
        .expect("a test runtime");
    let value = json(VALUE);
    cache
        .set_cache(runtime.handle(), "scope-1", &value, &kwargs())
        .expect("the write");
    let mut found = metadata();
    let cached = cache
        .get_cache(runtime.handle(), "scope-1", &kwargs(), Some(&mut found))
        .expect("the read");
    assert_eq!(cached, Some(value));
    assert_eq!(similarity_of(&found), Some(0.9_f64));
    let payload = &transport.payloads()[0];
    assert_eq!(
        payload.get("litellm_cache_key"),
        Some(&Json::Text("scope-1".to_owned()))
    );
    assert_eq!(
        payload.get("text"),
        Some(&Json::Text("what is cached".to_owned()))
    );
    assert_eq!(payload.get("response"), Some(&Json::Text(VALUE.to_owned())));
}

#[tokio::test]
async fn qdrant_semantic_cache_async_roundtrip() {
    let transport = Arc::new(FakeQdrant::new(0.9_f64, Vec::new()));
    let embedder = Arc::new(ConstantEmbedder::new());
    let concrete = Arc::clone(&transport);
    let backend: Arc<dyn QdrantTransport> = concrete;
    let cache = cache_from(backend, Arc::clone(&embedder));
    let value = json(VALUE);
    let embed_metadata = metadata();
    let call_kwargs = QdrantCacheKwargs {
        messages: Some(vec![user_message("what is cached")]),
        metadata: Some(embed_metadata.clone()),
    };
    cache
        .async_set_cache("scope-1", &value, &call_kwargs)
        .await
        .expect("the asynchronous write");
    let mut found = metadata();
    let cached = cache
        .async_get_cache("scope-1", &call_kwargs, Some(&mut found))
        .await
        .expect("the asynchronous read");
    assert_eq!(cached, Some(value));
    assert_eq!(similarity_of(&found), Some(0.9_f64));
    let calls = embedder.calls();
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|(prompt, recorded)| prompt == "what is cached"
                && recorded.as_ref() == Some(&embed_metadata))
    );
}

#[tokio::test]
async fn sync_cache_calls_inside_a_runtime_are_refused() {
    let (transport, cache) = memory_cache(0.9);
    let handle = Handle::current();
    let mut found = metadata();
    let write = cache.set_cache(&handle, "scope-1", &json("{}"), &kwargs());
    assert!(matches!(write, Err(QdrantSemanticError::BlockingRuntime)));
    let read = cache.get_cache(&handle, "scope-1", &kwargs(), Some(&mut found));
    assert!(matches!(read, Err(QdrantSemanticError::BlockingRuntime)));
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn uuid_provider_generates_point_ids() {
    let (transport, cache) = memory_cache(0.9);
    cache
        .async_set_cache("scope-1", &json("{}"), &kwargs())
        .await
        .expect("the write");
    let body = body_of(&transport.requests()[0]);
    let Json::List(points) = field(&body, "points") else {
        panic!("a points request carries a list");
    };
    assert_eq!(
        field(&points[0], "id"),
        &Json::Text(Uuid::from_u128(FIXED_ID).to_string())
    );
}

#[tokio::test]
async fn store_sends_the_wait_query_and_the_embedding() {
    let (transport, cache) = memory_cache(0.9);
    cache
        .async_set_cache("scope-1", &json("{}"), &kwargs())
        .await
        .expect("the write");
    let request = &transport.requests()[0];
    assert_eq!(request.method, Method::PUT);
    assert_eq!(request.path, "/collections/semantic-cache/points");
    assert_eq!(request.query, vec![("wait".to_owned(), "true".to_owned())]);
    let body = body_of(request);
    let Json::List(points) = field(&body, "points") else {
        panic!("a points request carries a list");
    };
    assert_eq!(field(&points[0], "vector"), &json("[0.25,0.5,0.75]"));
}

#[tokio::test]
async fn store_rejects_a_rejected_point_write() {
    let transport = Arc::new(FakeQdrant::new(
        0.9_f64,
        vec![answer(400, r#"{"status":{"error":"wrong vector size"}}"#)],
    ));
    let cache = cache_with(Arc::clone(&transport), Arc::new(ConstantEmbedder::new()));
    let error = cache
        .async_set_cache("scope-1", &json("{}"), &kwargs())
        .await
        .expect_err("a rejected point write must fail");
    assert!(
        error
            .to_string()
            .contains("/collections/semantic-cache/points")
    );
    assert!(error.to_string().contains("400"));
    assert!(error.to_string().contains("wrong vector size"));
}

#[tokio::test]
async fn batch_write_propagates_a_rejected_point() {
    let transport = Arc::new(FakeQdrant::new(0.9_f64, vec![answer(500, "boom")]));
    let cache = cache_with(Arc::clone(&transport), Arc::new(ConstantEmbedder::new()));
    let entries = vec![("a".to_owned(), json("{}"))];
    let error = cache
        .async_set_cache_pipeline(&entries, &kwargs())
        .await
        .expect_err("a rejected point inside the batch must fail the pipeline");
    assert!(error.to_string().contains("500"));
    assert!(error.to_string().contains("boom"));
}

#[tokio::test]
async fn store_without_messages_writes_nothing() {
    let (transport, cache) = memory_cache(0.9);
    let empty = QdrantCacheKwargs {
        messages: None,
        metadata: None,
    };
    cache
        .async_set_cache("scope-1", &json("{}"), &empty)
        .await
        .expect("a write without messages");
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn store_embedding_failure_propagates() {
    let transport = Arc::new(FakeQdrant::new(0.9_f64, Vec::new()));
    let concrete = Arc::clone(&transport);
    let backend: Arc<dyn QdrantTransport> = concrete;
    let cache = cache_from(backend, Arc::new(ConstantEmbedder::failing()));
    let error = cache
        .async_set_cache("scope-1", &json("{}"), &kwargs())
        .await
        .expect_err("an embedding failure must fail the write");
    assert_eq!(error.to_string(), "the embedding provider failed");
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn batch_write_propagates_the_first_error() {
    let transport = Arc::new(FakeQdrant::new(
        0.9_f64,
        vec![answer(200, r#"{"result":true}"#), Reply::Fail],
    ));
    let cache = cache_with(Arc::clone(&transport), Arc::new(ConstantEmbedder::new()));
    let entries = vec![("a".to_owned(), json("{}")), ("b".to_owned(), json("{}"))];
    let error = cache
        .async_set_cache_pipeline(&entries, &kwargs())
        .await
        .expect_err("a failed write in the batch must fail the pipeline");
    assert!(matches!(error, QdrantSemanticError::Transport { .. }));
}

#[tokio::test]
async fn pipeline_writes_every_entry() {
    let transport = Arc::new(FakeQdrant::new(
        0.9_f64,
        vec![
            answer(200, r#"{"result":true}"#),
            answer(200, r#"{"result":true}"#),
        ],
    ));
    let cache = cache_with(Arc::clone(&transport), Arc::new(ConstantEmbedder::new()));
    let entries = vec![("a".to_owned(), json("{}")), ("b".to_owned(), json("{}"))];
    cache
        .async_set_cache_pipeline(&entries, &kwargs())
        .await
        .expect("the batch writes");
    let keys: Vec<String> = transport.requests().iter().map(stored_key).collect();
    assert_eq!(keys, ["a", "b"]);
}
