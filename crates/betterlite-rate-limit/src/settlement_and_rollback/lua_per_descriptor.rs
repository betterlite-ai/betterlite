use std::fmt::Display;
use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::limiter_records::{CounterRefund, DescriptorAtomicGroup, RateLimitResponse};
use crate::rate_limit_window_store::pipeline_scripts::pipeline_scripts;
use crate::rate_limit_window_store::{CounterScript, ScriptCall, ScriptHandle};

use super::dispatch::settle_in_memory;
use super::pipelined_settlement::settle_pipelined;
use super::sequential_loop::settle_sequentially;
use super::{AtomicSettlement, SettlementError};

pub(super) async fn lua_per_descriptor(
    settlement: &AtomicSettlement<'_>,
    groups: &[DescriptorAtomicGroup],
    script: &Arc<dyn CounterScript>,
) -> Result<RateLimitResponse, SettlementError> {
    let calls: Vec<ScriptCall> = groups
        .iter()
        .map(|(keys, args, _meta)| {
            let scalars: Vec<RedisScalar> = args
                .iter()
                .map(|value| RedisScalar::Text(value.to_string()))
                .collect();
            (keys.clone(), scalars)
        })
        .collect();
    let pipelined = pipeline_scripts(settlement.windows, script, &calls);
    let mut handles: Vec<ScriptHandle> = Vec::with_capacity(pipelined.len());
    for enqueued in pipelined {
        let Some(handle) = enqueued else {
            return settle_sequentially(settlement, groups, &calls, script.as_ref()).await;
        };
        handles.push(handle);
    }
    settle_pipelined(settlement, groups, handles).await
}

pub(super) async fn lua_failure_fallback<E>(
    settlement: &AtomicSettlement<'_>,
    groups: &[DescriptorAtomicGroup],
    applied: &[Vec<CounterRefund>],
    error: &E,
) -> Result<RateLimitResponse, SettlementError>
where
    E: Display + Sync + ?Sized,
{
    let message = error.to_string();
    settlement.refund_applied_descriptor_groups(applied).await;
    settlement
        .fail_closed
        .reject_if_unverifiable("check_and_increment_by_n_script", &message)?;
    tracing::error!(
        error = %message,
        script = "check_and_increment_by_n_script",
        applied = applied.len(),
        window_size = settlement.default_window_size,
        "the Redis Lua check-and-increment failed, the applied increments were refunded and the descriptors fall back to in-memory enforcement"
    );
    settle_in_memory(settlement, groups).await
}
