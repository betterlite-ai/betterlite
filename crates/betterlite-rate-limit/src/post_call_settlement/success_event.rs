use std::sync::PoisonError;

use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use crate::limiter_records::ReservationAwareIncrement;
use crate::reconciliation_operations::io_token::{IoTokenReconciliation, io_token_reservation_ops};
use crate::reconciliation_operations::success_event::{SuccessEvent, success_event_pipeline_ops};
use crate::request_stash::RequestStashSlot;
use crate::token_adjustments::direct::increment_tokens_keeping_ttl;
use crate::token_adjustments::router::apply_reservation_aware_tokens;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

use super::settlement::{PostCallSettlement, PostCallSettlementError};

pub struct SuccessCallback<'request> {
    pub event: SuccessEvent<'request>,
    pub io: IoTokenReconciliation<'request>,
}

pub(super) async fn log_success_event(
    settlement: &PostCallSettlement<'_>,
    callback: &SuccessCallback<'_>,
) {
    if let Err(error) = settle_success(settlement, callback).await {
        tracing::warn!(%error, "Error in rate limit success event");
    }
}

pub(super) async fn settle_success(
    settlement: &PostCallSettlement<'_>,
    callback: &SuccessCallback<'_>,
) -> Result<(), PostCallSettlementError> {
    let event = &callback.event;
    let stash = event.stash.for_call(event.call_id);
    settlement
        .gauges
        .release_stashed_parallel_slot(stash.as_deref(), true)
        .await;
    let tpm_operations = success_event_pipeline_ops(event)?;
    if !tpm_operations.is_empty() {
        increment_tokens_keeping_ttl(settlement.adjustments, &tpm_operations).await;
    }
    let io_operations = io_token_reservation_ops(event.stash, event.call_id, &callback.io)?;
    apply_io_operations(
        settlement.adjustments,
        event.stash,
        event.call_id,
        io_operations,
    )
    .await?;
    Ok(())
}

// the source applies a no-window io result as a plain TTL-preserving pipeline and the windowed one as guarded ops
async fn apply_io_operations(
    adjustments: &TokenAdjustments,
    slot: &RequestStashSlot,
    call_id: Option<&str>,
    operations: Vec<ReservationAwareIncrement>,
) -> Result<(), TokenAdjustmentError> {
    if operations.is_empty() {
        return Ok(());
    }
    if io_ops_are_plain(slot, call_id) {
        let plain: Vec<IncrementOperation> = operations
            .iter()
            .map(|operation| operation.operation.clone())
            .collect();
        increment_tokens_keeping_ttl(adjustments, &plain).await;
        return Ok(());
    }
    apply_reservation_aware_tokens(adjustments, &operations).await
}

fn io_ops_are_plain(slot: &RequestStashSlot, call_id: Option<&str>) -> bool {
    let Some(stash) = slot.for_call(call_id) else {
        return false;
    };
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.reservation_released
        || (state.itpm_reserved_windows.is_empty() && state.otpm_reserved_windows.is_empty())
}
