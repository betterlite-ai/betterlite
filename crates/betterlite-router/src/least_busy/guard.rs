use std::fmt;
use std::sync::Arc;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_value_store::{RouteIncrement, RouteKey, RouteValue, RoutingValueStore};

use crate::routing_state_store::error::Observed;

use super::LeastBusy;
use super::error::{LeastBusyWarning, UnwrittenCount};

// what one pre-call callback started: the release it owes, or none when the attempt names no counter
pub struct Started {
    pub call: Option<InFlightCall>,
    pub warning: Option<LeastBusyWarning>,
}

// the ownership guard: success, mapped failure or a cancelled stream consumes it, and a dropped one releases this worker's count
pub struct InFlightCall {
    service: Arc<LeastBusy>,
    key: RouteKey,
    released: bool,
}

impl InFlightCall {
    // the guard one pre-call hands back for an attempt that now holds a count
    pub(super) fn started(service: Arc<LeastBusy>, key: RouteKey) -> Self {
        Self {
            service,
            key,
            released: false,
        }
    }

    // log_success_event, log_failure_event and the closed stream all release the one call they own
    pub async fn release(mut self) -> Option<LeastBusyWarning> {
        self.released = true;
        self.service.increment(&self.key, -1).await
    }
}

impl fmt::Debug for InFlightCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InFlightCall")
            .field("key", &self.key)
            .field("released", &self.released)
            .finish_non_exhaustive()
    }
}

// whether this worker's counter took the write, and the warning a rejected one raised
pub(super) enum LocalWrite {
    Written,
    Rejected(LeastBusyWarning),
}

// increment_cache with local_only: a counter driven below zero is written back as zero
pub(super) fn write_local(
    store: &RoutingValueStore,
    key: &RouteKey,
    delta: i64,
    ttl: Option<f64>,
) -> LocalWrite {
    match store.increment(key, &RouteIncrement::Counter(delta), ttl) {
        Ok(RouteValue::Counter(value)) if value.is_negative() => {
            match store.set(key, &RouteValue::Counter(0), ttl) {
                Ok(()) => LocalWrite::Written,
                Err(source) => LocalWrite::Rejected(unwritable(key, UnwrittenCount::Local(source))),
            }
        }
        Ok(RouteValue::Counter(_)) => LocalWrite::Written,
        Ok(_) => LocalWrite::Rejected(unwritable(key, UnwrittenCount::NotACount)),
        Err(source) => LocalWrite::Rejected(unwritable(key, UnwrittenCount::Local(source))),
    }
}

// increment_with_floor: the corrective write that clamps a negative shared count is the second half of the floor
pub(super) async fn write_shared(
    store: &RoutingStateStore,
    key: &RouteKey,
    delta: i64,
    ttl: Option<f64>,
) -> Option<LeastBusyWarning> {
    let failed = match store
        .increment(key, &RouteIncrement::Counter(delta), ttl)
        .await
    {
        Ok(Observed { value, .. }) => match value {
            RouteValue::Counter(count) if count.is_negative() => {
                let floor = count.saturating_neg();
                store
                    .increment(key, &RouteIncrement::Counter(floor), ttl)
                    .await
                    .err()
                    .map(UnwrittenCount::Shared)
            }
            RouteValue::Counter(_) => None,
            RouteValue::Spend(_)
            | RouteValue::Pin(_)
            | RouteValue::SchedulerTuple(_)
            | RouteValue::Members(_) => Some(UnwrittenCount::NotACount),
        },
        Err(source) => Some(UnwrittenCount::Shared(source)),
    };
    failed.map(|cause| unwritable(key, cause))
}

fn unwritable(key: &RouteKey, cause: UnwrittenCount) -> LeastBusyWarning {
    LeastBusyWarning::Unwritable {
        key: key.as_ref().to_owned(),
        cause,
    }
}

impl Drop for InFlightCall {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.service.release_locally(&self.key);
    }
}
