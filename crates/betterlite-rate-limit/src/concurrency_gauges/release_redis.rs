use crate::limiter_records::{ParallelSlotAcquisition, counter_values_from_reply};

use super::ConcurrencyGauges;
use super::lua_scripts::release_script_args;
use super::mirror_count::mirror_released_slots;
use super::release_in_memory::release_slots_in_memory;

mod tests;

pub(super) async fn release_request_slots(
    gauges: &ConcurrencyGauges,
    acquisition: &ParallelSlotAcquisition,
) {
    let counter_keys = &acquisition.counter_keys;
    let slot_id = &acquisition.slot_id;
    if counter_keys.is_empty() || slot_id.is_empty() {
        return;
    }
    if let Some(script) = gauges.release_script.as_deref() {
        let args = release_script_args(counter_keys, slot_id);
        match script.run(counter_keys, &args).await {
            Ok(reply) => match counter_values_from_reply(&reply) {
                Ok(remaining) => {
                    mirror_released_slots(gauges, counter_keys, &remaining).await;
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "parallel_release_script failed, falling back to in-memory release");
                }
            },
            Err(error) => {
                tracing::warn!(%error, "parallel_release_script failed, falling back to in-memory release");
            }
        }
    }
    release_slots_in_memory(gauges, counter_keys, slot_id).await;
}
