use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_refund::{
    BATCH_ENQUEUED_REFUND_STATUSES, BatchEnqueuedRefundError,
};
use betterlite_core::api::batch_enqueued_reserve::provider::canonical_provider_batch_id;
use betterlite_models::call_helpers::headers::AdditionalHeaders;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::reconciliation_operations::ReconciliationError;
use crate::reconciliation_operations::io_token::io_token_reservation_ops;
use crate::reconciliation_operations::success_event::success_event_pipeline_ops;
use crate::request_stash::{RequestStash, RequestStashSlot};
use crate::token_adjustments::direct::increment_tokens_keeping_ttl;
use crate::token_adjustments::router::apply_reservation_aware_tokens;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};
use crate::usage_resolution::recovered_partial_usage_tokens;

use super::apply::{apply_pipeline_operations, apply_side_operations};
use super::batch_enqueued::{BatchEnqueuedCall, BatchEnqueuedTokenStore, status_text};
use super::failure_event::FailureEvent;
use super::failure_hook::FailureHook;
use super::refund_operations::reserved_operations;
use super::snapshot::{
    batch_enqueued_reservation, batch_tpd_refund_ops, clear_batch_reservation,
    clear_batch_tpd_refund_ops, mark_reservation_released, stash_snapshot,
};
use super::success_event::{SuccessCallback, apply_io_operations};
use super::success_hook::{SuccessHook, merge_response_headers};

// the limiter fields the post-call callbacks settle against, shared by every callback below
pub struct PostCallSettlement<'dependencies> {
    pub gauges: &'dependencies ConcurrencyGauges,
    pub adjustments: &'dependencies TokenAdjustments,
    pub batch_store: &'dependencies dyn BatchEnqueuedTokenStore,
}

#[derive(Debug, thiserror::Error)]
pub enum PostCallSettlementError {
    #[error(transparent)]
    Reconciliation(#[from] ReconciliationError),
    #[error(transparent)]
    Tokens(#[from] TokenAdjustmentError),
    #[error(transparent)]
    Batch(#[from] BatchEnqueuedRefundError),
}

// the workspace denies multiple_inherent_impl, so every callback's body lives in this one impl block
impl PostCallSettlement<'_> {
    // LiteLLM's async_post_call_success_hook: release the slot, merge headers, run the fallible batch step
    pub async fn post_call_success_hook(
        &self,
        hook: &SuccessHook<'_>,
        response_headers: Option<&mut AdditionalHeaders>,
    ) {
        let slot_stash = hook.stash.for_call(hook.call_id);
        self.gauges
            .release_stashed_parallel_slot(slot_stash.as_deref(), false)
            .await;
        let header_stash = hook.stash.get();
        merge_response_headers(response_headers, header_stash.as_deref());
        let batch = BatchEnqueuedCall {
            stash: hook.stash,
            response: hook.response,
        };
        if let Err(error) = self.handle_batch_post_call(&batch).await {
            tracing::warn!(%error, "Error in batch enqueued-token post-call hook");
        }
    }

    pub async fn log_success_event(&self, callback: &SuccessCallback<'_>) {
        if let Err(error) = self.settle_success(callback).await {
            tracing::warn!(%error, "Error in rate limit success event");
        }
    }

    async fn settle_success(
        &self,
        callback: &SuccessCallback<'_>,
    ) -> Result<(), PostCallSettlementError> {
        let event = &callback.event;
        let stash = event.stash.for_call(event.call_id);
        self.gauges
            .release_stashed_parallel_slot(stash.as_deref(), true)
            .await;
        let tpm_operations = success_event_pipeline_ops(event)?;
        if !tpm_operations.is_empty() {
            increment_tokens_keeping_ttl(self.adjustments, &tpm_operations).await;
        }
        let io_operations = io_token_reservation_ops(event.stash, event.call_id, &callback.io)?;
        apply_io_operations(self.adjustments, event.stash, event.call_id, io_operations).await?;
        Ok(())
    }

    pub async fn log_failure_event(&self, event: &FailureEvent<'_>) {
        if let Err(error) = self.settle_failure(event).await {
            tracing::warn!(%error, "Error in rate limit failure event");
        }
    }

    async fn settle_failure(
        &self,
        event: &FailureEvent<'_>,
    ) -> Result<(), PostCallSettlementError> {
        let call_stash = event.stash.for_call(event.call_id);
        self.gauges
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
        apply_pipeline_operations(self.adjustments, &operations.combined).await;
        apply_side_operations(self.adjustments, operations.itpm).await?;
        apply_side_operations(self.adjustments, operations.otpm).await?;
        if amounts.0 > 0 || amounts.1 > 0 || amounts.2 > 0 {
            mark_reservation_released(&stash);
        }
        Ok(())
    }

    pub async fn post_call_failure_hook(&self, event: &FailureHook<'_>) {
        if let Err(error) = self.settle_failure_hook(event).await {
            tracing::warn!(%error, "Error releasing TPM reservation on post-call failure");
        }
    }

    async fn settle_failure_hook(
        &self,
        event: &FailureHook<'_>,
    ) -> Result<(), PostCallSettlementError> {
        let Some(stash) = event.stash.get() else {
            return Ok(());
        };
        self.gauges
            .release_stashed_parallel_slot(Some(stash.as_ref()), false)
            .await;
        self.settle_released_reservations(&stash).await?;
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
        apply_pipeline_operations(self.adjustments, &operations.combined).await;
        apply_side_operations(self.adjustments, operations.itpm).await?;
        apply_side_operations(self.adjustments, operations.otpm).await?;
        mark_reservation_released(&stash);
        Ok(())
    }

    // the batch reservation and the batch TPD refunds run before the release guard, since neither is a TPM reservation
    async fn settle_released_reservations(
        &self,
        stash: &RequestStash,
    ) -> Result<(), PostCallSettlementError> {
        if let Some(reservation) = batch_enqueued_reservation(stash) {
            self.batch_store
                .refund(&ReservationRecord::from(&reservation))
                .await?;
            clear_batch_reservation(stash);
        }
        let operations = batch_tpd_refund_ops(stash);
        if !operations.is_empty() {
            apply_reservation_aware_tokens(self.adjustments, &operations).await?;
            clear_batch_tpd_refund_ops(stash);
        }
        Ok(())
    }

    // LiteLLM's disconnect hook frees the stashed acquisition, not what the key's limits say now
    pub async fn release_slot_on_disconnect(&self, slot: &RequestStashSlot) {
        let stash = slot.get();
        self.gauges
            .release_stashed_parallel_slot(stash.as_deref(), false)
            .await;
    }

    // LiteLLM's batch post-call: save under the canonical batch id, then refund on a terminal status
    pub async fn handle_batch_post_call(
        &self,
        call: &BatchEnqueuedCall<'_>,
    ) -> Result<(), BatchEnqueuedRefundError> {
        let Some(view) = call.response else {
            return Ok(());
        };
        let batch_id = canonical_provider_batch_id(&view.id);
        if let Some(stash) = call.stash.get()
            && let Some(reservation) = batch_enqueued_reservation(&stash)
        {
            self.batch_store
                .save_reservation(&batch_id, &reservation)
                .await?;
            clear_batch_reservation(&stash);
        }
        if !BATCH_ENQUEUED_REFUND_STATUSES.contains(&status_text(view.status)) {
            return Ok(());
        }
        let Some(popped) = self.batch_store.pop_reservation(&batch_id).await? else {
            return Ok(());
        };
        self.batch_store.refund(&popped).await
    }
}
