use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use betterlite_redis::request_batch::lock;
use tokio::time::{Instant, sleep};
use tokio_util::task::TaskTracker;

use crate::reservation_settlement::handle::ReservationState;

use super::ports::ReservationLeases;

// the spend counter Redis backend the renewal refreshes through, with the TTL-derived interval and the request deadline
pub struct LeaseRenewal<'services> {
    pub leases: Arc<dyn ReservationLeases>,
    pub interval: Duration,
    pub request_timeout: Duration,
    pub tasks: &'services TaskTracker,
}

// _start_reservation_lease_renewal: one task, a weak record, the frozen key set, and no strong reference that outlives the request
pub fn start(
    renewal: &LeaseRenewal<'_>,
    state: Weak<Mutex<ReservationState>>,
    counter_keys: BTreeSet<String>,
) {
    let interval = renewal.interval;
    let deadline = renewal_deadline(renewal.request_timeout);
    let leases = Arc::clone(&renewal.leases);
    let _task = renewal.tasks.spawn(async move {
        renew(state, counter_keys, interval, deadline, leases).await;
    });
}

// litellm.request_timeout from the monotonic clock; a timeout too large to place ends the renewal at once
fn renewal_deadline(request_timeout: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(request_timeout).unwrap_or(now)
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
