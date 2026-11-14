use std::slice;

use crate::atomic_counter::response::build_atomic_response;
use crate::limiter_records::{DescriptorAtomicGroup, RateLimitResponse, counter_values_from_reply};
use crate::rate_limit_window_store::{CounterScript, ScriptCall};

use super::lua_per_descriptor::lua_failure_fallback;
use super::{AtomicSettlement, SettlementError};

mod tests;

pub(super) async fn settle_single_descriptor(
    settlement: &AtomicSettlement<'_>,
    group: &DescriptorAtomicGroup,
    call: &ScriptCall,
    script: &dyn CounterScript,
) -> Result<RateLimitResponse, SettlementError> {
    let (_keys, _args, meta) = group;
    let reply = match script.run(&call.0, &call.1).await {
        Ok(reply) => reply,
        Err(error) => {
            return lua_failure_fallback(settlement, slice::from_ref(group), &[], &error).await;
        }
    };
    let values = counter_values_from_reply(&reply)?;
    Ok(build_atomic_response(&values, meta)?)
}
