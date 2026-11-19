use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::local_ttl_cache as local_config;
use betterlite_config::types::Config;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::routing_state_store::Settings as SharedSettings;
use betterlite_router::routing_state_store::error::StateStoreError;
use betterlite_router::routing_state_store::{RedisSettings, RoutingStateStore};
use betterlite_router::routing_value_store::{Clock, StoreSettings};

use crate::routing::redis_failure_policy::policy_settings;

#[cfg(test)]
mod tests;

// Router.cache for deployment affinity: this worker's pins always, and the shared pins when Redis is configured
#[must_use]
pub fn build(config: &Config) -> Option<Arc<AffinityClaims>> {
    let clock = wall_clock();
    let local = match LocalPins::new(local_settings(&config.sections.local_ttl_cache), clock) {
        Ok(local) => local,
        Err(error) => {
            tracing::error!(%error, "the affinity in-memory cache settings are unreadable");
            return None;
        }
    };
    Some(Arc::new(AffinityClaims {
        local,
        shared: shared_store(config),
    }))
}

// the router's InMemoryCache defaults: 200 items, a 600-second default ttl and 1024 KiB per item
fn local_settings(section: &local_config::Settings) -> StoreSettings {
    StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: Some(1024),
        loaded_max_size_per_item_kib: section.global_max_item_kib,
    }
}

// a shared store that cannot be built leaves this worker's pins alone, as a router without Redis keeps them
fn shared_store(config: &Config) -> Option<RoutingStateStore> {
    let settings = shared_settings(config)?;
    let opened = Instant::now();
    match RoutingStateStore::new(&settings, Arc::new(move || opened.elapsed())) {
        Ok(store) => Some(store),
        Err(error) => {
            tracing::error!(%error, "the affinity shared pin store cannot be built");
            None
        }
    }
}

// router_settings.redis_url or redis_host and redis_port: a config naming neither keeps no shared pins
fn shared_settings(config: &Config) -> Option<SharedSettings> {
    match RedisSettings::try_from(&config.sections.routing_state_store) {
        Ok(redis) => Some(SharedSettings {
            redis,
            breaker: policy_settings(&config.sections.redis_failure_policy),
        }),
        Err(StateStoreError::NotConfigured) => None,
        Err(error) => {
            tracing::error!(%error, "the affinity shared pin settings are unreadable");
            None
        }
    }
}

// the local tier's ttl counts from wall time, while the breaker measures the time since it opened
fn wall_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0_f64, |since| since.as_secs_f64())
    })
}
