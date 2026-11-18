#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_config::sections::model_rate_limits as section;
use betterlite_router::io_token_reservations::clock::IoClock;
use betterlite_router::model_rate_limits::cache::RateLimitCache;
use betterlite_router::routing_value_store::{RoutingValueStore, StoreSettings};

use super::{ModelRateLimitStores, RoutingRateLimitCache, build};

fn local() -> Arc<RoutingValueStore> {
    let settings = StoreSettings {
        max_size_in_memory: Some(200),
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    };
    Arc::new(
        RoutingValueStore::new(settings, Arc::new(|| 0.0)).expect("the store settings are valid"),
    )
}

fn stores(local: Arc<RoutingValueStore>) -> ModelRateLimitStores {
    ModelRateLimitStores {
        local,
        remote: None,
    }
}

fn cache(local: Arc<RoutingValueStore>) -> RoutingRateLimitCache {
    RoutingRateLimitCache {
        local,
        remote: None,
    }
}

fn clock() -> IoClock {
    Arc::new(|| UNIX_EPOCH + Duration::from_secs(1_700_000_000))
}

#[test]
fn build_requires_the_config_name() {
    let disabled = section::Settings::default();
    assert!(build(&disabled, stores(local()), None, clock()).is_none());
    let enabled = section::Settings {
        enforce_model_rate_limits: true,
    };
    assert!(build(&enabled, stores(local()), None, clock()).is_some());
}

#[test]
fn the_local_tier_counts_in_the_store() {
    let local = local();
    let cache = cache(Arc::clone(&local));
    assert_eq!(cache.increment_local("router:tpm", 2, 60), Some(2));
    assert_eq!(cache.increment_local("router:tpm", 3, 60), Some(5));
    assert_eq!(cache.local("router:tpm"), Some(5));
    assert_eq!(cache.local("router:absent"), None);
}

#[test]
fn an_absent_remote_keeps_the_local_count() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds");
    let cache = cache(local());
    assert!(!cache.has_remote());
    assert_eq!(
        runtime.block_on(cache.increment("router:rpm", 1, 60)),
        Some(1)
    );
    assert_eq!(
        runtime.block_on(cache.remote("router:rpm")),
        super::RemoteRead::Value(None)
    );
    assert_eq!(cache.local("router:rpm"), Some(1));
}

#[test]
fn an_oversized_key_is_not_counted() {
    let cache = cache(local());
    let key = "k".repeat(2048);
    assert_eq!(cache.local(&key), None);
    assert_eq!(cache.increment_local(&key, 1, 60), None);
}
