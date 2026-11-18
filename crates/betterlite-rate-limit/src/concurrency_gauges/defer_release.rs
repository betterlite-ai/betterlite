use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;

use crate::limiter_records::ParallelSlotAcquisition;

use super::ConcurrencyGauges;
use super::batch::ReleaseSettledHook;
use super::lua_scripts::release_script_args;
use super::release_in_memory::release_slots_in_memory;

mod tests;

pub(super) struct LogReleaseFailure;

impl ReleaseSettledHook for LogReleaseFailure {
    fn settled(&self, failure: Option<&BatchError>) {
        let Some(error) = failure else {
            return;
        };
        tracing::warn!(%error, "parallel_release_script failed, the slot stays released in memory only");
    }
}

pub(super) async fn defer_slot_release(
    gauges: &ConcurrencyGauges,
    acquisition: &ParallelSlotAcquisition,
) -> bool {
    let Some(script) = gauges.release_script.as_ref() else {
        return false;
    };
    let Some(batch) = gauges
        .release_batch
        .as_ref()
        .and_then(|source| source.active_release_batch())
    else {
        return false;
    };
    if acquisition.counter_keys.is_empty() || acquisition.slot_id.is_empty() {
        return false;
    }
    release_slots_in_memory(gauges, &acquisition.counter_keys, &acquisition.slot_id).await;
    let args = release_script_args(&acquisition.counter_keys, &acquisition.slot_id);
    batch.register_release(
        Arc::clone(script),
        &acquisition.counter_keys,
        &args,
        Arc::new(LogReleaseFailure),
    );
    true
}
