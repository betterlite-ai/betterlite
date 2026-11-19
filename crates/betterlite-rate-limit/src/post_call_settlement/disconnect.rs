use crate::request_stash::RequestStashSlot;

use super::settlement::PostCallSettlement;

// LiteLLM's disconnect hook frees the stashed acquisition, not what the key's limits say now
pub(super) async fn release_slot_on_disconnect(
    settlement: &PostCallSettlement<'_>,
    slot: &RequestStashSlot,
) {
    let stash = slot.get();
    settlement
        .gauges
        .release_stashed_parallel_slot(stash.as_deref(), false)
        .await;
}
