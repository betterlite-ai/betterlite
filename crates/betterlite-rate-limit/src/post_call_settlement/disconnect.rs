use crate::request_stash::RequestStashSlot;

use super::settlement::PostCallSettlement;

impl PostCallSettlement<'_> {
    // LiteLLM's disconnect hook frees the stashed acquisition, not what the key's limits say now
    pub async fn release_slot_on_disconnect(&self, slot: &RequestStashSlot) {
        let stash = slot.get();
        self.gauges
            .release_stashed_parallel_slot(stash.as_deref(), false)
            .await;
    }
}
