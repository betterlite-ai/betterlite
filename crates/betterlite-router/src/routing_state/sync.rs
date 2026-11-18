use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use crate::router_observations::counter::{CounterError, Increment};
use crate::routing_value_store::RouteKey;

use super::FLUSH_DEADLINE;
use super::counter::RouterCounters;
use super::delta::{self, Delta, DeltaStore, SyncOutcome};

impl<S: DeltaStore + 'static> RouterCounters<S> {
    // BaseRoutingStrategy._sync_in_memory_spend_with_redis
    pub async fn sync_with_redis(&self) -> bool {
        matches!(
            delta::sync_state(self.state.as_ref()).await,
            SyncOutcome::Pushed
        )
    }

    // the queue bound is the owner's batch size: a full queue flushes once in the caller's timeframe
    pub(super) async fn flush_or_drop(&self, delta: Delta) {
        let key = delta.key.as_ref().to_owned();
        let outcome =
            tokio::time::timeout(FLUSH_DEADLINE, delta::sync_state(self.state.as_ref())).await;
        let queued = match outcome {
            Ok(SyncOutcome::Pushed | SyncOutcome::Empty) => self.state.queue(delta),
            Ok(SyncOutcome::Failed | SyncOutcome::Busy) | Err(_elapsed) => false,
        };
        if queued {
            return;
        }
        tracing::warn!(
            key = key.as_str(),
            "the queue is full; the delta is dropped after the bounded flush"
        );
    }

    // B397's synchronous CounterSink cannot await redis, so the delta joins the batch or waits in the overflow
    pub(super) fn queue_usage(&self, increment: &Increment) -> Result<(), CounterError> {
        let route_key =
            RouteKey::parse(&increment.key).map_err(|_source| CounterError::Rejected {
                key: increment.key.clone(),
            })?;
        if self.state.store.is_some() {
            self.state.track(&route_key);
        }
        if self
            .state
            .enqueue_usage(&route_key, increment.value, increment.ttl)?
        {
            self.schedule_usage_flush();
        }
        Ok(())
    }

    // one flush task at a time drains the overflow; the flag and its guard keep a stuck store from piling up tasks
    fn schedule_usage_flush(&self) {
        if self.state.flushing_usage.swap(true, Ordering::AcqRel) {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            self.state.flushing_usage.store(false, Ordering::Release);
            tracing::warn!("no runtime is running; the usage overflow waits for the next sync");
            return;
        }
        let state = Arc::clone(&self.state);
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .spawn(delta::flush_usage_overflow(state));
    }
}
