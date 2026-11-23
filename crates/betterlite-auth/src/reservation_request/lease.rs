use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use betterlite_redis::request_batch::lock;
use tokio::time::{Instant, sleep};
use tokio_util::task::TaskTracker;

use crate::reservation_settlement::handle::ReservationState;

use super::ports::ReservationLeases;

// the spend counter cache's Redis backend, whose default TTL halves into the interval, and the request deadline
pub struct LeaseRenewal<'services> {
    pub leases: Arc<dyn ReservationLeases>,
    pub redis_default_ttl: i64,
    pub request_timeout: Duration,
    pub tasks: &'services TaskTracker,
}

// _start_reservation_lease_renewal: one task, a weak record, the frozen key set, and no strong reference that outlives the request
pub fn start(
    renewal: &LeaseRenewal<'_>,
    state: Weak<Mutex<ReservationState>>,
    counter_keys: BTreeSet<String>,
) {
    let Some(interval) = renewal_interval(renewal.redis_default_ttl) else {
        return;
    };
    // litellm.request_timeout from the monotonic clock; a timeout too large to place ends the renewal at once
    let now = Instant::now();
    let deadline = now.checked_add(renewal.request_timeout).unwrap_or(now);
    let leases = Arc::clone(&renewal.leases);
    let _task = renewal.tasks.spawn(async move {
        renew(state, counter_keys, interval, deadline, leases).await;
    });
}

// BaseCache.default_ttl halved, in milliseconds so an odd TTL keeps its half second
#[must_use]
pub fn renewal_interval(default_ttl: i64) -> Option<Duration> {
    let ttl = u64::try_from(default_ttl).ok()?;
    if ttl == 0 {
        return None;
    }
    let half_millis = ttl.checked_mul(500)?;
    Some(Duration::from_millis(half_millis))
}

// _renew_reservation_lease: sleep first and check after, so a request shorter than half the TTL never refreshes
async fn renew(
    state: Weak<Mutex<ReservationState>>,
    counter_keys: BTreeSet<String>,
    interval: Duration,
    deadline: Instant,
    leases: Arc<dyn ReservationLeases>,
) {
    while Instant::now() < deadline {
        sleep(interval).await;
        let Some(record) = state.upgrade() else {
            return;
        };
        let finalized = lock(&record).finalized;
        drop(record);
        if finalized {
            return;
        }
        for counter_key in &counter_keys {
            leases.refresh(counter_key).await;
        }
    }
}
