use crate::request_stash::RequestStashSlot;

use super::settlement::PostCallSettlement;

impl PostCallSettlement<'_> {
    // LiteLLM's async_release_max_parallel_requests_on_disconnect: the stashed acquisition decides the release,
    // not the key's current configuration, so a request that changed limits mid-flight still frees its slot
    pub async fn release_max_parallel_requests_on_disconnect(&self, slot: &RequestStashSlot) {
        let stash = slot.get();
        self.gauges
            .release_stashed_parallel_slot(stash.as_deref(), false)
            .await;
    }
}
