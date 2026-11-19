use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;

use crate::request_stash::{RequestStash, RequestStashSlot};
use crate::token_adjustments::router::apply_reservation_aware_tokens;
use crate::usage_resolution::{Usage, recovered_partial_usage_tokens};

use super::apply::{apply_pipeline_operations, apply_side_operations};
use super::refund_operations::reserved_operations;
use super::settlement::{PostCallSettlement, PostCallSettlementError};
use super::snapshot::{
    batch_enqueued_reservation, batch_tpd_refund_ops, clear_batch_reservation,
    clear_batch_tpd_refund_ops, mark_reservation_released, stash_snapshot,
};

pub struct FailureHook<'request> {
    pub stash: &'request RequestStashSlot,
    pub combined_usage: Option<&'request Usage>,
    pub rate_limit_type: TokenRateLimitType,
    pub window_size: i64,
}

pub(super) async fn post_call_failure_hook(
    settlement: &PostCallSettlement<'_>,
    event: &FailureHook<'_>,
) {
    if let Err(error) = settle_failure_hook(settlement, event).await {
        tracing::warn!(%error, "Error releasing TPM reservation on post-call failure");
    }
}

pub(super) async fn settle_failure_hook(
    settlement: &PostCallSettlement<'_>,
    event: &FailureHook<'_>,
) -> Result<(), PostCallSettlementError> {
    let Some(stash) = event.stash.get() else {
        return Ok(());
    };
    settlement
        .gauges
        .release_stashed_parallel_slot(Some(stash.as_ref()), false)
        .await;
    settle_released_reservations(settlement, &stash).await?;
    let snapshot = stash_snapshot(&stash);
    if snapshot.reservation_released {
        return Ok(());
    }
    let amounts = (
        snapshot.reserved_tokens,
        snapshot.itpm_reserved_tokens,
        snapshot.otpm_reserved_tokens,
    );
    if amounts.0 <= 0 && amounts.1 <= 0 && amounts.2 <= 0 {
        return Ok(());
    }
    let actual = recovered_partial_usage_tokens(event.combined_usage, event.rate_limit_type);
    let operations = reserved_operations(&snapshot, amounts, actual, event.window_size)?;
    apply_pipeline_operations(settlement.adjustments, &operations.combined).await;
    apply_side_operations(settlement.adjustments, operations.itpm).await?;
    apply_side_operations(settlement.adjustments, operations.otpm).await?;
    mark_reservation_released(&stash);
    Ok(())
}

// the batch reservation and the batch TPD refunds run before the release guard, since neither is a TPM reservation
async fn settle_released_reservations(
    settlement: &PostCallSettlement<'_>,
    stash: &RequestStash,
) -> Result<(), PostCallSettlementError> {
    if let Some(reservation) = batch_enqueued_reservation(stash) {
        settlement
            .batch_store
            .refund(&ReservationRecord::from(&reservation))
            .await?;
        clear_batch_reservation(stash);
    }
    let operations = batch_tpd_refund_ops(stash);
    if !operations.is_empty() {
        apply_reservation_aware_tokens(settlement.adjustments, &operations).await?;
        clear_batch_tpd_refund_ops(stash);
    }
    Ok(())
}
