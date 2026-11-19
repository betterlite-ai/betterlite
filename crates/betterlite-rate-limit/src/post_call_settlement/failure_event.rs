use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::request_stash::RequestStashSlot;
use crate::usage_resolution::{Usage, recovered_partial_usage_tokens};

use super::apply::{apply_pipeline_operations, apply_side_operations};
use super::refund_operations::reserved_operations;
use super::settlement::{PostCallSettlement, PostCallSettlementError};
use super::snapshot::{mark_reservation_released, stash_snapshot};

pub struct FailureEvent<'request> {
    pub stash: &'request RequestStashSlot,
    pub call_id: Option<&'request str>,
    pub combined_usage: Option<&'request Usage>,
    pub rate_limit_type: TokenRateLimitType,
    pub window_size: i64,
}

pub(super) async fn log_failure_event(
    settlement: &PostCallSettlement<'_>,
    event: &FailureEvent<'_>,
) {
    if let Err(error) = settle_failure(settlement, event).await {
        tracing::warn!(%error, "Error in rate limit failure event");
    }
}

pub(super) async fn settle_failure(
    settlement: &PostCallSettlement<'_>,
    event: &FailureEvent<'_>,
) -> Result<(), PostCallSettlementError> {
    let call_stash = event.stash.for_call(event.call_id);
    settlement
        .gauges
        .release_stashed_parallel_slot(call_stash.as_deref(), true)
        .await;
    let Some(stash) = call_stash else {
        return Ok(());
    };
    let snapshot = stash_snapshot(&stash);
    let amounts = if snapshot.reservation_released {
        (0, 0, 0)
    } else {
        (
            snapshot.reserved_tokens,
            snapshot.itpm_reserved_tokens,
            snapshot.otpm_reserved_tokens,
        )
    };
    let actual = recovered_partial_usage_tokens(event.combined_usage, event.rate_limit_type);
    let operations = reserved_operations(&snapshot, amounts, actual, event.window_size)?;
    apply_pipeline_operations(settlement.adjustments, &operations.combined).await;
    apply_side_operations(settlement.adjustments, operations.itpm).await?;
    apply_side_operations(settlement.adjustments, operations.otpm).await?;
    if amounts.0 > 0 || amounts.1 > 0 || amounts.2 > 0 {
        mark_reservation_released(&stash);
    }
    Ok(())
}
