mod tests;

use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;
use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_openai::batches::records::BatchStatus;

use crate::request_stash::RequestStashSlot;

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

// the declared status, spelled as the lowercase string BATCH_ENQUEUED_REFUND_STATUSES carries
pub(super) const fn status_text(status: BatchStatus) -> &'static str {
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
