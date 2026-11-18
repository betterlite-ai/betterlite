use std::mem;
use std::sync::{Arc, PoisonError, Weak};
use std::time::Duration;

use tokio::time::sleep;

use super::FLUSH_DEADLINE;
use super::counter::RouterCounters;
use super::delta::{self, DeltaStore};

impl<S: DeltaStore + 'static> RouterCounters<S> {
    // BaseRoutingStrategy.__init__ starts the periodic sync only for a strategy that batches its writes
    pub fn start(self: &Arc<Self>) {
        if self.settings.should_batch_redis_writes {
            self.setup_sync_task();
        }
    }

    // BaseRoutingStrategy.setup_sync_task
    pub fn setup_sync_task(self: &Arc<Self>) {
        let interval = self.settings.resolved_sync_interval();
        let owner = Arc::downgrade(self);
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        tasks.abort_all();
        while tasks.try_join_next().is_some() {}
        tasks.spawn(run_periodic(owner, interval));
    }

    // BaseRoutingStrategy.cancel_sync_task
    pub fn cancel_sync_task(&self) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .abort_all();
    }

    // BaseRoutingStrategy.cleanup cancels the sync and waits for it to stop
    pub async fn cleanup(&self) {
        self.cancel_sync_task();
        let mut tasks = mem::take(&mut *self.tasks.lock().unwrap_or_else(PoisonError::into_inner));
        while tasks.join_next().await.is_some() {}
    }

    // BaseRoutingStrategy.retire stops the loop, then pushes what is queued inside the flush deadline
    pub async fn retire(&self) {
        self.cancel_sync_task();
        let Some(_gate) = self.state.gate.acquire(FLUSH_DEADLINE).await else {
            tracing::warn!("a routing flush held the store past the retire deadline");
            return;
        };
        let _pushed = delta::push_pending(self.state.as_ref()).await;
    }
}

async fn run_periodic<S: DeltaStore + 'static>(owner: Weak<RouterCounters<S>>, interval: Duration) {
    while let Some(counters) = owner.upgrade() {
        let _synced = counters.sync_with_redis().await;
        drop(counters);
        sleep(interval).await;
    }
}
