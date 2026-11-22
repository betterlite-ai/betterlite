use crate::routing::redis_failure_policy::policy_settings;
use crate::routing::routing_value_store::local_store;
use betterlite_config::types::Config;
use betterlite_router::request_scheduler::service::{RequestIdSource, RequestScheduler};
use betterlite_router::routing_state_store::Settings as SharedSettings;
use betterlite_router::routing_state_store::error::StateStoreError;
use betterlite_router::routing_state_store::{RedisSettings, RoutingStateStore};
use betterlite_router::routing_value_store::Clock;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
mod tests;
// Router.scheduler: this worker's queue always, and the queue Redis shares when the config names a server
#[must_use]
pub fn build(config: &Config) -> Option<Arc<RequestScheduler>> {
    let local = match local_store(&config.sections.local_ttl_cache, wall_clock()) {
        Ok(local) => local,
        Err(error) => {
            tracing::error!(%error, "the scheduler queue's in-memory cache settings are unreadable");
            return None;
        }
    };
    Some(Arc::new(RequestScheduler::new(
        local,
        shared_store(config),
        config.sections.request_scheduler.clone(),
        request_ids(),
    )))
}
// Router.schedule_acompletion's `str(uuid.uuid4())`
fn request_ids() -> RequestIdSource {
    Arc::new(|| {
        let generated = uuid::Uuid::new_v4();
        generated.to_string()
    })
}
// a shared queue that cannot be built leaves this worker's queue alone, as a router without Redis keeps it
fn shared_store(config: &Config) -> Option<RoutingStateStore> {
    let redis = match RedisSettings::try_from(&config.sections.routing_state_store) {
        Ok(redis) => redis,
        Err(StateStoreError::NotConfigured) => return None,
        Err(error) => {
            tracing::error!(%error, "the scheduler queue's shared settings are unreadable");
            return None;
        }
    };
    let settings = SharedSettings {
        redis,
        breaker: policy_settings(&config.sections.redis_failure_policy),
    };
    let opened = Instant::now();
    match RoutingStateStore::new(&settings, Arc::new(move || opened.elapsed())) {
        Ok(store) => Some(store),
        Err(error) => {
            tracing::error!(%error, "the scheduler queue's shared store cannot be built");
            None
        }
    }
}
// the shared queue's ttl counts from wall time, while the breaker measures the time since it opened
fn wall_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs_f64()
    })
}
