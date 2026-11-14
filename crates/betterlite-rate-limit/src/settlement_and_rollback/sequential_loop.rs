use std::collections::BTreeSet;

use crate::atomic_counter::response::build_atomic_response;
use crate::limiter_records::{
    CounterRefund, DescriptorAtomicGroup, RateLimitResponse, RateLimitStatus,
    ReservationWindowIdentity, counter_values_from_reply,
};
use crate::rate_limit_window_store::{CounterScript, ScriptCall};

// ast-grep-ignore: no-long-names
use super::refunds::counter_refunds_from_atomic_response as atomic_response_refunds;
use super::{AtomicSettlement, SettlementError};

mod tests;

impl AtomicSettlement<'_> {
    pub(super) async fn settle_sequentially(
        &self,
        groups: &[DescriptorAtomicGroup],
        calls: &[ScriptCall],
        script: &dyn CounterScript,
    ) -> Result<RateLimitResponse, SettlementError> {
        if let ([group], [call]) = (groups, calls) {
            return self.settle_single_descriptor(group, call, script).await;
        }
        let mut applied: Vec<Vec<CounterRefund>> = Vec::new();
        let mut statuses: Vec<RateLimitStatus> = Vec::new();
        let mut windows: BTreeSet<ReservationWindowIdentity> = BTreeSet::new();
        for ((_keys, _args, meta), call) in groups.iter().zip(calls) {
            let reply = match script.run(&call.0, &call.1).await {
                Ok(reply) => reply,
                Err(error) => return self.lua_failure_fallback(groups, &applied, &error).await,
            };
            let values = counter_values_from_reply(&reply)?;
            let response = build_atomic_response(&values, meta)?;
            if response.overall_code == "OVER_LIMIT" {
                self.refund_applied_descriptor_groups(&applied).await;
                return Ok(response);
            }
            applied.push(atomic_response_refunds(&values, meta)?);
            statuses.extend(response.statuses);
            windows.extend(response.reservation_windows.into_iter().flatten());
        }
        Ok(RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses,
            reservation_windows: Some(windows),
        })
    }
}
