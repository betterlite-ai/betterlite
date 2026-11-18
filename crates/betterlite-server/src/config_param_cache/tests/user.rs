use betterlite_cache::dual_cache::CacheOptions;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_models::decisions::Json;

use super::support::{FakeStore, Harness, user_row};
use crate::config_param_cache::{USER_ROW_KEY_SUFFIX, cache_user_row};

#[tokio::test]
async fn a_miss_reads_the_user_row_and_caches_its_json_text() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_user("u1");
    let cache = harness.cache(60.0);

    cache_user_row("u1", &cache, &store)
        .await
        .expect("the user row caches");
    assert_eq!(store.user_calls(), 1);
    let key = format!("u1{USER_ROW_KEY_SUFFIX}");
    let cached = harness.dual.async_get_cache(&key, false).await;
    let cached_row = cached.expect("the cached JSON text is decoded by the memory tier");
    assert!(matches!(&cached_row, Json::Object(_)));
    let decoded: LiteLLMUserTable =
        serde_json::from_str(&serde_json::to_string(&cached_row).unwrap())
            .expect("the cached row decodes");
    assert_eq!(decoded.user_id, "u1");
    cache_user_row("u1", &cache, &store)
        .await
        .expect("the cached user row is reused");
    assert_eq!(store.user_calls(), 1);
}

#[tokio::test]
async fn the_cached_user_row_expires_after_ten_minutes() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_user("u1");
    let cache = harness.cache(60.0);

    cache_user_row("u1", &cache, &store)
        .await
        .expect("the user row caches");
    harness.advance(600);
    cache_user_row("u1", &cache, &store)
        .await
        .expect("the cached user row is reused");
    assert_eq!(store.user_calls(), 1);
    harness.advance(1);
    cache_user_row("u1", &cache, &store)
        .await
        .expect("the user row caches again");
    assert_eq!(store.user_calls(), 2);
}

#[tokio::test]
async fn an_absent_user_is_not_cached() {
    let harness = Harness::new();
    let store = FakeStore::default();
    let cache = harness.cache(60.0);

    cache_user_row("u1", &cache, &store)
        .await
        .expect("the absence is not an error");
    assert_eq!(store.user_calls(), 1);
    let key = format!("u1{USER_ROW_KEY_SUFFIX}");
    assert!(harness.dual.async_get_cache(&key, false).await.is_none());
    cache_user_row("u1", &cache, &store)
        .await
        .expect("the absence is not an error");
    assert_eq!(store.user_calls(), 2);
}

#[tokio::test]
async fn a_cached_user_row_skips_the_store() {
    let harness = Harness::new();
    let store = FakeStore::default();
    let cache = harness.cache(60.0);
    let key = format!("u1{USER_ROW_KEY_SUFFIX}");
    let text = serde_json::to_string(&user_row("u1")).expect("the user row serializes");
    harness.dual.set_cache(
        &key,
        &Json::Text(text),
        CacheOptions {
            ttl: Some(600.0_f64),
            local_only: false,
            refresh_ttl: false,
        },
    );

    cache_user_row("u1", &cache, &store)
        .await
        .expect("the cached user row is reused");
    assert_eq!(store.user_calls(), 0);
}
