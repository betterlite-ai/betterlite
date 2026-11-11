#![cfg(test)]

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;

use super::super::support_tests::{
    DetailBody, FakeCache, FakeLive, StatusBody, admin_router, call_json, fake_cache,
};

#[tokio::test]
async fn a_redis_cache_deletes_the_posted_keys() {
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.delete = Mutex::new(Some(Ok(())));
    let case = Arc::new(cache);
    let (status, body): (StatusCode, StatusBody) = call_json(
        admin_router(FakeLive::with(Arc::<FakeCache>::clone(&case))),
        "POST",
        "/cache/delete",
        r#"{"keys": ["k1", "k2"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.status, "success");
    let deleted = case.deleted.lock().unwrap();
    assert_eq!(*deleted, vec![vec!["k1".to_owned(), "k2".to_owned()]]);
}

#[tokio::test]
async fn an_empty_key_list_still_reaches_the_cache() {
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    let case = Arc::new(cache);
    let (status, _body): (StatusCode, StatusBody) = call_json(
        admin_router(FakeLive::with(Arc::<FakeCache>::clone(&case))),
        "POST",
        "/cache/delete",
        r#"{"keys": []}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(*case.deleted.lock().unwrap(), vec![Vec::<String>::new()]);
}

#[tokio::test]
async fn an_uninitialized_cache_answers_500() {
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::without()),
        "POST",
        "/cache/delete",
        r#"{"keys": ["k1"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body.detail,
        "Cache Delete Failed(Cache not initialized. litellm.cache is None)"
    );
}

#[tokio::test]
async fn a_non_redis_cache_answers_500() {
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(fake_cache()))),
        "POST",
        "/cache/delete",
        r#"{"keys": ["k1"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body.detail,
        "Cache Delete Failed(Cache type local does not support deleting a key. only `redis` is supported)"
    );
}

#[tokio::test]
async fn a_redis_delete_failure_answers_500() {
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.delete = Mutex::new(Some(Err(())));
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(cache))),
        "POST",
        "/cache/delete",
        r#"{"keys": ["k1"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body.detail,
        "Cache Delete Failed(deleting the cache keys failed)"
    );
}

#[tokio::test]
async fn absent_keys_answer_500() {
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    let case = Arc::new(cache);
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::<FakeCache>::clone(&case))),
        "POST",
        "/cache/delete",
        r#"{"keys": null}"#,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.detail, "Cache Delete Failed(keys is required)");
    assert!(case.deleted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_malformed_body_answers_500() {
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(fake_cache()))),
        "POST",
        "/cache/delete",
        "{not json",
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.detail.starts_with("Cache Delete Failed("));
}

#[tokio::test]
async fn keys_of_the_wrong_type_answer_500() {
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    let case = Arc::new(cache);
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::<FakeCache>::clone(&case))),
        "POST",
        "/cache/delete",
        r#"{"keys": 7}"#,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.detail.starts_with("Cache Delete Failed("));
    assert!(case.deleted.lock().unwrap().is_empty());
}
