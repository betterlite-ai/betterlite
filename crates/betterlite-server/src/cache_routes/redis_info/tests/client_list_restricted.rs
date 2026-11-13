#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache_admin::info_parse::InfoValue;
use indexmap::IndexMap;

use super::super::admin::RedisAdmin;
use super::super::redis_support::{FakeRedis, fake_redis};
use super::super::support_tests::{
    FakeLive, RedisClientsBody, RedisInfoBody, admin_router, call_json, fake_cache,
};

#[tokio::test]
async fn client_list_restricted_returns_the_warning_and_minus_one() {
    let redis = Arc::new(fake_redis());
    *redis.clients.lock().unwrap() = Err(());
    let mut fields = IndexMap::new();
    fields.insert(
        "redis_version".to_owned(),
        InfoValue::Text("7.2.4".to_owned()),
    );
    *redis.info.lock().unwrap() = Ok(fields);
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
    assert_eq!(body.num_clients, -1);
    let RedisClientsBody::Unavailable(clients) = body.clients else {
        panic!("the restricted client list is the warning");
    };
    assert_eq!(
        clients,
        vec!["CLIENT LIST command not available on this Redis instance".to_owned()]
    );
    assert_eq!(
        body.info.get("redis_version"),
        Some(&Json::Text("7.2.4".to_owned()))
    );
}
