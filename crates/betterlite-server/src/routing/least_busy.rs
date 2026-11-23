use std::sync::Arc;

use betterlite_config::types::Config;
use betterlite_router::least_busy::LeastBusy;

use crate::routing::routing_value_store::{local_store, shared_store, wall_clock};

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
        shared: shared_store(
            config,
            "the least-busy shared counter settings are unreadable",
            "the least-busy shared counter store cannot be built",
        ),
    }))
}
