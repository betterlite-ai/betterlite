use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::local_ttl_cache as local_ttl;
use betterlite_config::types::Config;
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::routing_state_store::Settings as SharedSettings;
use betterlite_router::routing_state_store::error::StateStoreError;
use betterlite_router::routing_state_store::{RedisSettings, RoutingStateStore};
use betterlite_router::routing_value_store::{Clock, RoutingValueStore, StoreError, StoreSettings};

mod tests;

pub fn local_store(
    section: &local_ttl::Settings,
    clock: Clock,
) -> Result<RoutingValueStore, StoreError> {
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: Some(1024),
            loaded_max_size_per_item_kib: section.global_max_item_kib,
        },
        clock,
    )
}

// router_settings.redis_url or redis_host and redis_port: a config naming neither keeps no shared tier
pub fn shared_settings(config: &Config, unreadable_message: &str) -> Option<SharedSettings> {
    match RedisSettings::try_from(&config.sections.routing_state_store) {
        Ok(redis) => Some(SharedSettings {
            redis,
            breaker: BreakerSettings::from(&config.sections.redis_failure_policy),
        }),
        Err(StateStoreError::NotConfigured) => None,
        Err(error) => {
            tracing::error!(%error, "{unreadable_message}");
            None
        }
    }
}

// a shared store that cannot be built leaves the caller's own tier alone, as a router without Redis keeps it
pub fn shared_store(
    config: &Config,
    unreadable_message: &str,
    build_failed_message: &str,
) -> Option<RoutingStateStore> {
    let settings = shared_settings(config, unreadable_message)?;
    let opened = Instant::now();
    match RoutingStateStore::new(&settings, Arc::new(move || opened.elapsed())) {
        Ok(store) => Some(store),
        Err(error) => {
            tracing::error!(%error, "{build_failed_message}");
            None
        }
    }
}

// the local tier's ttl counts from wall time, while the breaker measures the time since it opened
#[must_use]
pub fn wall_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0_f64, |since| since.as_secs_f64())
    })
}
