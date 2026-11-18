use std::sync::PoisonError;

use crate::limiter_records::ParallelSlotAcquisition;
use crate::request_stash::RequestStash;

use super::ConcurrencyGauges;
use super::defer_release::defer_slot_release;
use super::release_redis::release_request_slots;

mod tests;

pub(super) async fn release_stashed_slot(
    gauges: &ConcurrencyGauges,
    stash: &RequestStash,
    in_logging_callback: bool,
) {
    let _guard = stash.parallel_slot_release_lock.lock().await;
    let acquisition = stashed_acquisition(stash);
    let Some(acquisition) = acquisition else {
        return;
    };
    let deferred = in_logging_callback && defer_slot_release(gauges, &acquisition).await;
    if !deferred {
        release_request_slots(gauges, &acquisition).await;
    }
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .parallel_slot = None;
}

fn stashed_acquisition(stash: &RequestStash) -> Option<ParallelSlotAcquisition> {
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    let acquisition = state.parallel_slot.clone();
    drop(state);
    acquisition
}
