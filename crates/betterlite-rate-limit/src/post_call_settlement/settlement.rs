use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;
use betterlite_models::call_helpers::headers::AdditionalHeaders;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::reconciliation_operations::ReconciliationError;
use crate::request_stash::RequestStashSlot;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

use super::batch_enqueued::{self, BatchEnqueuedCall, BatchEnqueuedTokenStore};
use super::disconnect;
use super::failure_event::{self, FailureEvent};
use super::failure_hook::{self, FailureHook};
use super::success_event::{self, SuccessCallback};
use super::success_hook::{self, SuccessHook};

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

// the workspace denies multiple_inherent_impl, so each callback's logic is a free function in its module and this is the type's one impl block
impl PostCallSettlement<'_> {
    pub async fn post_call_success_hook(
        &self,
        hook: &SuccessHook<'_>,
        response_headers: Option<&mut AdditionalHeaders>,
    ) {
        success_hook::post_call_success_hook(self, hook, response_headers).await;
    }

    pub async fn log_success_event(&self, callback: &SuccessCallback<'_>) {
        success_event::log_success_event(self, callback).await;
    }

    pub(crate) async fn settle_success(
        &self,
        callback: &SuccessCallback<'_>,
    ) -> Result<(), PostCallSettlementError> {
        success_event::settle_success(self, callback).await
    }

    pub async fn log_failure_event(&self, event: &FailureEvent<'_>) {
        failure_event::log_failure_event(self, event).await;
    }

    pub(crate) async fn settle_failure(
        &self,
        event: &FailureEvent<'_>,
    ) -> Result<(), PostCallSettlementError> {
        failure_event::settle_failure(self, event).await
    }

    pub async fn post_call_failure_hook(&self, event: &FailureHook<'_>) {
        failure_hook::post_call_failure_hook(self, event).await;
    }

    pub(crate) async fn settle_failure_hook(
        &self,
        event: &FailureHook<'_>,
    ) -> Result<(), PostCallSettlementError> {
        failure_hook::settle_failure_hook(self, event).await
    }

    pub async fn release_slot_on_disconnect(&self, slot: &RequestStashSlot) {
        disconnect::release_slot_on_disconnect(self, slot).await;
    }

    pub async fn handle_batch_post_call(
        &self,
        call: &BatchEnqueuedCall<'_>,
    ) -> Result<(), BatchEnqueuedRefundError> {
        batch_enqueued::handle_batch_post_call(self, call).await
    }
}
