use std::sync::PoisonError;

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
    let acquisition = {
        let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.parallel_slot.clone()
    };
    let Some(acquisition) = acquisition else {
        return;
    };
    let deferred = in_logging_callback && defer_slot_release(gauges, &acquisition).await;
    if !deferred {
        release_request_slots(gauges, &acquisition).await;
    }
    let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.parallel_slot = None;
}
