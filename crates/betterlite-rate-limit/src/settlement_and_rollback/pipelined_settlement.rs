use std::collections::BTreeSet;

use crate::limiter_records::{
    CounterRefund, DescriptorAtomicGroup, RateLimitResponse, RateLimitStatus,
    ReservationWindowIdentity,
};
use crate::rate_limit_window_store::ScriptHandle;

use super::group_response::{GroupResponse, pipelined_group_response};
use super::lua_per_descriptor::lua_failure_fallback;

mod tests;
// ast-grep-ignore: no-long-names
use super::refunds::counter_refunds_from_atomic_response as atomic_response_refunds;
use super::{AtomicSettlement, SettlementError};

struct AppliedGroups {
    refunds: Vec<Vec<CounterRefund>>,
    statuses: Vec<RateLimitStatus>,
    windows: BTreeSet<ReservationWindowIdentity>,
}

pub(super) async fn settle_pipelined(
    settlement: &AtomicSettlement<'_>,
    groups: &[DescriptorAtomicGroup],
    handles: Vec<ScriptHandle>,
) -> Result<RateLimitResponse, SettlementError> {
    let mut responses: Vec<GroupResponse> = Vec::with_capacity(groups.len());
    for (handle, (_keys, _args, meta)) in handles.into_iter().zip(groups) {
        responses.push(pipelined_group_response(handle.await, meta));
    }
    let applied = accumulate_applied_groups(&responses, groups)?;
    if let Some(response) = responses.iter().find_map(GroupResponse::over_limit) {
        settlement
            .refund_applied_descriptor_groups(&applied.refunds)
            .await;
        return Ok(response.clone());
    }
    if let Some(failure) = responses.iter().find_map(GroupResponse::failure) {
        return lua_failure_fallback(settlement, groups, &applied.refunds, failure).await;
    }
    if let [GroupResponse::Response { response, .. }] = responses.as_slice() {
        return Ok(response.clone());
    }
    Ok(RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses: applied.statuses,
        reservation_windows: Some(applied.windows),
    })
}

fn accumulate_applied_groups(
    responses: &[GroupResponse],
    groups: &[DescriptorAtomicGroup],
) -> Result<AppliedGroups, SettlementError> {
    let mut applied = AppliedGroups {
        refunds: Vec::new(),
        statuses: Vec::new(),
        windows: BTreeSet::new(),
    };
    for (entry, (_keys, _args, meta)) in responses.iter().zip(groups) {
        let GroupResponse::Response { response, values } = entry else {
            continue;
        };
        if response.overall_code != "OK" {
            continue;
        }
        applied.refunds.push(atomic_response_refunds(values, meta)?);
        applied.statuses.extend(response.statuses.iter().cloned());
        applied
            .windows
            .extend(response.reservation_windows.iter().flatten().cloned());
    }
    Ok(applied)
}
