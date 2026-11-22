use crate::routing::routing_value_store::{local_store, shared_store, wall_clock};
use betterlite_config::types::Config;
use betterlite_router::request_scheduler::service::{RequestIdSource, RequestScheduler};
use std::sync::Arc;
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
        shared_store(
            config,
            "the scheduler queue's shared settings are unreadable",
            "the scheduler queue's shared store cannot be built",
        ),
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
