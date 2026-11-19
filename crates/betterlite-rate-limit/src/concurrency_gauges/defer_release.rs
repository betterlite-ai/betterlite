use std::sync::Arc;

use betterlite_redis::batch::result::{Settled, SettledFollowUp, SettledHook};
use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::limiter_records::{ParallelSlotAcquisition, ScriptReply};

use super::ConcurrencyGauges;
use super::release_in_memory::release_slots_in_memory;

mod tests;

pub(super) async fn defer_slot_release(
    gauges: &ConcurrencyGauges,
    acquisition: &ParallelSlotAcquisition,
) -> bool {
    let Some(script) = gauges.release_script.as_ref() else {
        return false;
    };
    let Some(batch) = gauges.release_batch.as_ref() else {
        return false;
    };
    if acquisition.counter_keys.is_empty() || acquisition.slot_id.is_empty() {
        return false;
    }
    release_slots_in_memory(gauges, &acquisition.counter_keys, &acquisition.slot_id).await;
    let args = vec![RedisScalar::Text(acquisition.slot_id.clone()); acquisition.counter_keys.len()];
    let hook: SettledHook<ScriptReply> = Arc::new(
        |settled: &Settled<ScriptReply>| -> Option<SettledFollowUp> {
            let Err(failure) = settled else {
                return None;
            };
            let error = failure.as_ref();
            tracing::warn!(%error, "parallel_release_script failed, the slot stays released in memory only");
            None
        },
    );
    batch.script_on_settled(Arc::clone(script), &acquisition.counter_keys, &args, hook);
    true
}
