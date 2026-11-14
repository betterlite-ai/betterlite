use std::fmt::Display;
use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::limiter_records::{CounterRefund, DescriptorAtomicGroup, RateLimitResponse};
use crate::rate_limit_window_store::{CounterScript, ScriptCall, ScriptHandle, pipeline_scripts};

use super::{AtomicSettlement, SettlementError};

impl AtomicSettlement<'_> {
    pub(super) async fn lua_per_descriptor(
        &self,
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
        let pipelined = pipeline_scripts(self.windows, script, &calls);
        let mut handles: Vec<ScriptHandle> = Vec::with_capacity(pipelined.len());
        for handle in pipelined {
            let Some(handle) = handle else {
                return self
                    .settle_sequentially(groups, &calls, script.as_ref())
                    .await;
            };
            handles.push(handle);
        }
        self.settle_pipelined(groups, handles).await
    }

    pub(super) async fn lua_failure_fallback<E>(
        &self,
        groups: &[DescriptorAtomicGroup],
        applied: &[Vec<CounterRefund>],
        error: &E,
    ) -> Result<RateLimitResponse, SettlementError>
    where
        E: Display + ?Sized,
    {
        self.refund_applied_descriptor_groups(applied).await;
        self.fail_closed
            .reject_if_unverifiable("check_and_increment_by_n_script", error)?;
        tracing::error!(
            %error,
            script = "check_and_increment_by_n_script",
            applied = applied.len(),
            window_size = self.default_window_size,
            "the Redis Lua check-and-increment failed, the applied increments were refunded and the descriptors fall back to in-memory enforcement"
        );
        self.settle_in_memory(groups).await
    }
}
