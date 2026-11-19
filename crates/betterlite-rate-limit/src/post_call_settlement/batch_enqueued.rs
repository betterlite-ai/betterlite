mod tests;

use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_refund::{
    BATCH_ENQUEUED_REFUND_STATUSES, BatchEnqueuedRefundError,
};
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use betterlite_core::api::batch_enqueued_reserve::provider::canonical_provider_batch_id;
use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_openai::batches::records::BatchStatus;

use crate::request_stash::RequestStashSlot;

use super::settlement::PostCallSettlement;
use super::snapshot::{batch_enqueued_reservation, clear_batch_enqueued_reservation};

// the store methods LiteLLM's batch_enqueued_token_store exposes to the limiter; B1043's EnqueuedTokenRefund implements them
#[async_trait::async_trait]
pub trait BatchEnqueuedTokenStore: Send + Sync {
    async fn save_reservation(
        &self,
        batch_id: &str,
        reservation: &BatchEnqueuedTokenReservation,
    ) -> Result<(), BatchEnqueuedRefundError>;

    async fn pop_reservation(
        &self,
        batch_id: &str,
    ) -> Result<Option<ReservationRecord>, BatchEnqueuedRefundError>;

    async fn refund(&self, reservation: &ReservationRecord)
    -> Result<(), BatchEnqueuedRefundError>;
}

pub struct BatchEnqueuedCall<'request> {
    pub stash: &'request RequestStashSlot,
    pub response: Option<&'request BatchResponseView>,
}

impl PostCallSettlement<'_> {
    // LiteLLM's _handle_batch_enqueued_post_call: save the request's reservation under the canonical batch id,
    // then, on a terminal status, pop and refund whatever the store still holds
    pub async fn handle_batch_enqueued_post_call(
        &self,
        call: &BatchEnqueuedCall<'_>,
    ) -> Result<(), BatchEnqueuedRefundError> {
        let Some(view) = call.response else {
            return Ok(());
        };
        let batch_id = canonical_provider_batch_id(&view.id);
        if let Some(stash) = call.stash.get() {
            if let Some(reservation) = batch_enqueued_reservation(&stash) {
                self.batch_store
                    .save_reservation(&batch_id, &reservation)
                    .await?;
                clear_batch_enqueued_reservation(&stash);
            }
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

// the declared status, spelled as the lowercase string BATCH_ENQUEUED_REFUND_STATUSES carries
const fn status_text(status: BatchStatus) -> &'static str {
    match status {
        BatchStatus::Validating => "validating",
        BatchStatus::Failed => "failed",
        BatchStatus::InProgress => "in_progress",
        BatchStatus::Finalizing => "finalizing",
        BatchStatus::Completed => "completed",
        BatchStatus::Expired => "expired",
        BatchStatus::Cancelling => "cancelling",
        BatchStatus::Cancelled => "cancelled",
    }
}
