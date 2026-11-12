#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache_admin::info_parse::InfoValue;
use indexmap::IndexMap;

use super::super::admin::RedisAdmin;
use super::super::redis_support::{FakeRedis, PanickingRedis, fake_redis};
use super::super::support_tests::{
    DetailBody, FakeLive, RedisClientsBody, RedisInfoBody, admin_router, call_json, fake_cache,
};

fn client(addr: &str) -> IndexMap<String, String> {
    let mut entry = IndexMap::new();
    entry.insert("addr".to_owned(), addr.to_owned());
    entry
}

fn info() -> IndexMap<String, InfoValue> {
    let mut fields = IndexMap::new();
    fields.insert(
        "redis_version".to_owned(),
        InfoValue::Text("7.2.4".to_owned()),
    );
    fields.insert("uptime_in_seconds".to_owned(), InfoValue::Int(42_i64));
    fields
}

#[tokio::test]
async fn the_redis_state_is_returned_with_the_client_list() {
    let redis = Arc::new(fake_redis());
    *redis.clients.lock().unwrap() = Ok(vec![client("127.0.0.1:1"), client("127.0.0.1:2")]);
    *redis.info.lock().unwrap() = Ok(info());
    let admin: Arc<dyn RedisAdmin> = Arc::<FakeRedis>::clone(&redis);
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.redis = Some(admin);
    let (status, body): (StatusCode, RedisInfoBody) = call_json(
        admin_router(FakeLive::with(Arc::new(cache))),
        "GET",
        "/cache/redis/info",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.num_clients, 2);
    let RedisClientsBody::Listed(clients) = body.clients else {
        panic!("the client list is parsed");
    };
    assert_eq!(clients[0]["addr"], "127.0.0.1:1");
    assert_eq!(clients[1]["addr"], "127.0.0.1:2");
    assert_eq!(
        body.info.get("redis_version"),
        Some(&Json::Text("7.2.4".to_owned()))
    );
    assert_eq!(
        body.info.get("uptime_in_seconds"),
        Some(&Json::Number(serde_json::Number::from(42_i64)))
    );
}

#[tokio::test]
async fn an_uninitialized_cache_answers_503() {
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::without()),
        "GET",
        "/cache/redis/info",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.detail,
        "Service Unhealthy (Cache not initialized. litellm.cache is None)"
    );
}

#[tokio::test]
async fn a_non_redis_cache_answers_503() {
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(fake_cache()))),
        "GET",
        "/cache/redis/info",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.detail,
        "Service Unhealthy (Cache type local does not support redis info)"
    );
}

#[tokio::test]
async fn an_info_failure_answers_503() {
    let redis = Arc::new(fake_redis());
    *redis.info.lock().unwrap() = Err(());
    let admin: Arc<dyn RedisAdmin> = Arc::<FakeRedis>::clone(&redis);
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.redis = Some(admin);
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(cache))),
        "GET",
        "/cache/redis/info",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.detail,
        "Service Unhealthy (reading the redis info failed)"
    );
}

#[tokio::test]
async fn a_panicking_info_answers_503() {
    let admin: Arc<dyn RedisAdmin> = Arc::new(PanickingRedis);
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.redis = Some(admin);
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(cache))),
        "GET",
        "/cache/redis/info",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body.detail
            .starts_with("Service Unhealthy (a blocking cache call did not finish")
    );
}
