use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::model_rate_limits as section;
use betterlite_router::io_token_reservations::clock::IoClock;
use betterlite_router::model_rate_limits::cache::{RateLimitCache, RemoteRead};
use betterlite_router::model_rate_limits::service::{IoTokenReservationHook, ModelRateLimits};
use betterlite_router::routing_state_store::RoutingStateStore;
use betterlite_router::routing_state_store::error::{StateStoreError, StoreWarning};
use betterlite_router::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

mod tests;

// The stores a router already owns: its in-memory routing values and, when redis_settings is set, the Redis tier
pub struct ModelRateLimitStores {
    pub local: Arc<RoutingValueStore>,
    pub remote: Option<Arc<RoutingStateStore>>,
}

// Router.add_optional_pre_call_checks registers ModelRateLimitingCheck only when the config names enforce_model_rate_limits
#[must_use]
pub fn build(
    section: &section::Settings,
    stores: ModelRateLimitStores,
    io: Option<Arc<dyn IoTokenReservationHook>>,
    clock: IoClock,
) -> Option<ModelRateLimits> {
    if !section.enforce_model_rate_limits {
        return None;
    }
    let cache = Arc::new(RoutingRateLimitCache {
        local: stores.local,
        remote: stores.remote,
    });
    Some(ModelRateLimits::new(cache, io, clock))
}

struct RoutingRateLimitCache {
    local: Arc<RoutingValueStore>,
    remote: Option<Arc<RoutingStateStore>>,
}

// The check counts in the router's own routing-value keyspace, so its keys and the strategy's counters stay apart
#[async_trait]
impl RateLimitCache for RoutingRateLimitCache {
    fn local(&self, key: &str) -> Option<i64> {
        let key = RouteKey::parse(key).ok()?;
        match self.local.get(&key).ok()? {
            StoreRead::Value(RouteValue::Counter(value)) => Some(value),
            StoreRead::Value(_) | StoreRead::Missing => None,
        }
    }

    fn has_remote(&self) -> bool {
        self.remote.is_some()
    }

    async fn remote(&self, key: &str) -> RemoteRead {
        let Some(store) = &self.remote else {
            return RemoteRead::Value(None);
        };
        let Ok(key) = RouteKey::parse(key) else {
            return RemoteRead::Failed;
        };
        // RedisCache.async_get_cache swallows a connection failure and returns None, so a failed read is an absent value
        match store.get(&key).await {
            Ok(observed) => match (observed.value, observed.warning) {
                (StoreRead::Missing, Some(StoreWarning::Unavailable { .. })) => {
                    RemoteRead::Value(None)
                }
                (StoreRead::Value(RouteValue::Counter(value)), _) => RemoteRead::Value(Some(value)),
                (StoreRead::Missing, _) => RemoteRead::Value(None),
                (StoreRead::Value(_), _) => RemoteRead::Failed,
            },
            Err(StateStoreError::CircuitOpen { .. }) => RemoteRead::BreakerOpen,
            Err(_) => RemoteRead::Failed,
        }
    }

    fn increment_local(&self, key: &str, by: i64, ttl_seconds: u64) -> Option<i64> {
        let key = RouteKey::parse(key).ok()?;
        match self.local.increment(
            &key,
            &RouteIncrement::Counter(by),
            Some(Duration::from_secs(ttl_seconds).as_secs_f64()),
        ) {
            Ok(RouteValue::Counter(value)) => Some(value),
            Ok(_) | Err(_) => None,
        }
    }

    async fn increment(&self, key: &str, by: i64, ttl_seconds: u64) -> Option<i64> {
        let local = self.increment_local(key, by, ttl_seconds);
        let Some(store) = &self.remote else {
            return local;
        };
        let Ok(key) = RouteKey::parse(key) else {
            return local;
        };
        let counter = store
            .increment(
                &key,
                &RouteIncrement::Counter(by),
                Some(Duration::from_secs(ttl_seconds).as_secs_f64()),
            )
            .await;
        match counter.map(|observed| observed.value) {
            Ok(RouteValue::Counter(value)) => Some(value),
            Ok(_) | Err(_) => local,
        }
    }
}
