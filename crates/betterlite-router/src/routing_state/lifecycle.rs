use std::sync::Weak;
use std::time::Duration;

use tokio::time::sleep;

use super::counter::RouterCounters;
use super::delta::DeltaStore;

// BaseRoutingStrategy.periodic_sync_in_memory_spend_with_redis
pub(super) async fn run_periodic<S>(owner: Weak<RouterCounters<S>>, interval: Duration)
where
    S: DeltaStore + 'static,
{
    while let Some(counters) = owner.upgrade() {
        let _synced = counters.sync_with_redis().await;
        drop(counters);
        sleep(interval).await;
    }
}
