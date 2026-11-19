use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::reconciliation_operations::ReconciliationError;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

use super::batch_enqueued::BatchEnqueuedTokenStore;

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
