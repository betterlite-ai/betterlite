use std::convert::Infallible;
use std::sync::Mutex;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::{ConfigParamInvalidator, TableOpsError};
use betterlite_models::decisions::Json;

use super::support::{FakeStore, Harness};
use crate::config_param_cache::store::ConfigParamCacheInvalidator;
use crate::config_param_cache::{
    CONFIG_CACHE_PREFIX, ConfigChangeFuture, ConfigChangePublisher, ConfigParamCacheError,
    MISS_MARKER,
};

struct OrderPublisher<'view> {
    cache: &'view DualCache<LocalMemoryOps>,
    key: String,
    seen: Mutex<Vec<(String, bool)>>,
}

impl ConfigChangePublisher for OrderPublisher<'_> {
    fn publish_param_change<'call>(
        &'call self,
        param_name: &'call str,
    ) -> ConfigChangeFuture<'call> {
        Box::pin(async move {
            let present = self.cache.async_get_cache(&self.key, true).await.is_some();
            self.seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((param_name.to_owned(), present));
        })
    }
}

#[tokio::test]
async fn a_miss_reads_the_store_and_caches_the_row() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_param("settings", r#"{"value":1}"#);
    let cache = harness.cache(60.0);

    let first = cache
        .get(&store, "settings")
        .await
        .expect("the store read succeeds");
    assert_eq!(
        first.as_ref().map(|row| row.param_name.as_str()),
        Some("settings")
    );
    let second = cache
        .get(&store, "settings")
        .await
        .expect("the cached read succeeds");
    assert_eq!(second, first);
    assert_eq!(store.param_calls(), 1);
    let cached = harness
        .dual
        .async_get_cache("litellm_config:param:settings", false)
        .await;
    assert!(matches!(cached, Some(Json::Object(_))));
}

#[tokio::test]
async fn an_absent_param_is_cached_as_the_miss_marker() {
    let harness = Harness::new();
    let store = FakeStore::default();
    let cache = harness.cache(60.0);

    assert!(
        cache
            .get(&store, "missing")
            .await
            .expect("the read succeeds")
            .is_none()
    );
    let key = format!("{CONFIG_CACHE_PREFIX}missing");
    let cached = harness.dual.async_get_cache(&key, false).await;
    assert_eq!(cached, Some(Json::Text(MISS_MARKER.to_owned())));
    assert!(
        cache
            .get(&store, "missing")
            .await
            .expect("the read succeeds")
            .is_none()
    );
    assert_eq!(store.param_calls(), 1);
}

#[tokio::test]
async fn the_cached_row_expires_after_the_ttl() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_param("settings", r#"{"value":1}"#);
    let cache = harness.cache(60.0);

    assert!(
        cache
            .get(&store, "settings")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    harness.advance(60);
    assert!(
        cache
            .get(&store, "settings")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    assert_eq!(store.param_calls(), 1);
    harness.advance(1);
    assert!(
        cache
            .get(&store, "settings")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    assert_eq!(store.param_calls(), 2);
}

#[tokio::test]
async fn evict_drops_the_cached_row() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_param("settings", r#"{"value":1}"#);
    let cache = harness.cache(60.0);

    assert!(
        cache
            .get(&store, "settings")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    cache
        .evict("settings")
        .await
        .expect("the eviction succeeds");
    assert!(
        cache
            .get(&store, "settings")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    assert_eq!(store.param_calls(), 2);
}

#[tokio::test]
async fn invalidate_evicts_before_it_publishes() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_param("settings", r#"{"value":1}"#);
    let cache = harness.cache(60.0);
    assert!(
        cache
            .get(&store, "settings")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    let publisher = OrderPublisher {
        cache: &harness.dual,
        key: format!("{CONFIG_CACHE_PREFIX}settings"),
        seen: Mutex::new(Vec::new()),
    };

    cache
        .invalidate("settings", &publisher)
        .await
        .expect("the invalidation succeeds");
    assert_eq!(
        *publisher
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![("settings".to_owned(), false)]
    );
    assert!(
        cache
            .get(&store, "settings")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    assert_eq!(store.param_calls(), 2);
}

#[tokio::test]
async fn the_insert_invalidator_evicts_before_it_publishes() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_param("settings", r#"{"value":1}"#);
    let cache = harness.cache(60.0);
    assert!(
        cache
            .get(&store, "settings")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    let publisher = OrderPublisher {
        cache: &harness.dual,
        key: format!("{CONFIG_CACHE_PREFIX}settings"),
        seen: Mutex::new(Vec::new()),
    };
    let invalidator = ConfigParamCacheInvalidator {
        cache: &cache,
        publisher: &publisher,
    };

    ConfigParamInvalidator::invalidate_param(&invalidator, "settings").await;
    assert_eq!(
        *publisher
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![("settings".to_owned(), false)]
    );
}

#[tokio::test]
async fn prefetch_reads_every_name_in_one_query() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_param("first", r#"{"value":1}"#);
    store.add_param("third", r#"{"value":3}"#);
    let cache = harness.cache(60.0);
    let names = vec!["first".to_owned(), "second".to_owned(), "third".to_owned()];

    cache.prefetch(&store, &names).await;
    assert_eq!(store.batch_calls(), 1);
    assert_eq!(
        *store
            .last_batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        names
    );
    assert!(
        cache
            .get(&store, "first")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    assert!(
        cache
            .get(&store, "second")
            .await
            .expect("the read succeeds")
            .is_none()
    );
    assert!(
        cache
            .get(&store, "third")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    assert_eq!(store.param_calls(), 0);
}

#[tokio::test]
async fn a_failed_prefetch_leaves_every_name_uncached() {
    let harness = Harness::new();
    let store = FakeStore::default();
    store.add_param("first", r#"{"value":1}"#);
    store.fail_batches();
    let cache = harness.cache(60.0);

    cache.prefetch(&store, &["first".to_owned()]).await;
    assert_eq!(store.batch_calls(), 1);
    assert!(
        cache
            .get(&store, "first")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    assert_eq!(store.param_calls(), 1);
}

#[tokio::test]
async fn an_empty_prefetch_reads_nothing() {
    let harness = Harness::new();
    let store = FakeStore::default();
    let cache = harness.cache(60.0);

    cache.prefetch(&store, &[]).await;
    assert_eq!(store.batch_calls(), 0);
}

#[test]
fn a_database_fault_stays_distinguishable_through_the_cache_error() {
    let no_db: ConfigParamCacheError<Infallible> =
        ConfigParamCacheError::from(TableOpsError::<Infallible>::NoDbConnected);
    assert!(matches!(
        no_db.database_source(),
        Some(DbError::NoDbConnected)
    ));
    let missing: ConfigParamCacheError<Infallible> =
        ConfigParamCacheError::from(TableOpsError::<Infallible>::MissingField {
            field: "param_name",
        });
    assert!(missing.database_source().is_none());
    assert!(matches!(missing, ConfigParamCacheError::Table { .. }));
}

#[tokio::test]
async fn object_cache_entries_round_trip_without_a_database_read() {
    let harness = Harness::new();
    let store = FakeStore::default();
    let value: Json =
        serde_json::from_str(r#"{"param_name":"settings","param_value":{"enabled":true}}"#)
            .unwrap();
    harness
        .dual
        .async_set_cache(
            "litellm_config:param:settings",
            &value,
            CacheOptions {
                ttl: Some(60.0),
                local_only: false,
                refresh_ttl: false,
            },
        )
        .await;
    let row = harness
        .cache(60.0)
        .get(&store, "settings")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.param_name, "settings");
    assert_eq!(
        serde_json::to_string(&row.param_value).unwrap(),
        r#"{"enabled":true}"#
    );
    assert_eq!(store.param_calls(), 0);
}
