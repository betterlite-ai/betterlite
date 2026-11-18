use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use betterlite_config::types::Config;
use betterlite_router::least_busy::LeastBusy;
use betterlite_router::routing_state_store::Settings as SharedSettings;
use betterlite_router::routing_state_store::error::StateStoreError;
use betterlite_router::routing_state_store::{RedisSettings, RoutingStateStore};
use betterlite_router::routing_value_store::Clock;

use crate::routing::redis_failure_policy::policy_settings;
use crate::routing::routing_value_store::local_store;

mod tests;

// Router.cache for the least-busy strategy: this worker's counters always, and the shared counters when Redis is configured
#[must_use]
pub fn build(config: &Config) -> Option<Arc<LeastBusy>> {
    let local = match local_store(&config.sections.local_ttl_cache, wall_clock()) {
        Ok(local) => local,
        Err(error) => {
            tracing::error!(%error, "the least-busy in-flight cache settings are unreadable");
            return None;
        }
    };
    Some(Arc::new(LeastBusy {
        local,
        shared: shared_store(config),
    }))
}

// a shared store that cannot be built leaves this worker's counters alone, as a router without Redis keeps them
fn shared_store(config: &Config) -> Option<RoutingStateStore> {
    let settings = shared_settings(config)?;
    let opened = Instant::now();
    match RoutingStateStore::new(&settings, Arc::new(move || opened.elapsed())) {
        Ok(store) => Some(store),
        Err(error) => {
            tracing::error!(%error, "the least-busy shared counter store cannot be built");
            None
        }
    }
}

// router_settings.redis_url or redis_host and redis_port: a config naming neither keeps no shared counters
fn shared_settings(config: &Config) -> Option<SharedSettings> {
    match RedisSettings::try_from(&config.sections.routing_state_store) {
        Ok(redis) => Some(SharedSettings {
            redis,
            breaker: policy_settings(&config.sections.redis_failure_policy),
        }),
        Err(StateStoreError::NotConfigured) => None,
        Err(error) => {
            tracing::error!(%error, "the least-busy shared counter settings are unreadable");
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
