use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use crate::token_adjustments::router::apply_reservation_aware_tokens;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

use super::refund_operations::SideOperations;

// the source's async_increment_cache_pipeline(_post_call): the request's dual-cache pipeline carries one batch
pub(crate) async fn apply_pipeline_operations(
    adjustments: &TokenAdjustments,
    operations: &[IncrementOperation],
) {
    if operations.is_empty() {
        return;
    }
    adjustments.usage.increment_pipeline(operations).await;
}

pub(crate) async fn apply_side_operations(
    adjustments: &TokenAdjustments,
    operations: SideOperations,
) -> Result<(), TokenAdjustmentError> {
    match operations {
        SideOperations::Empty => Ok(()),
        SideOperations::Plain(increments) => {
            if !increments.is_empty() {
                adjustments.usage.increment_pipeline(&increments).await;
            }
            Ok(())
        }
        SideOperations::ReservationAware(guarded) => {
            apply_reservation_aware_tokens(adjustments, &guarded).await
        }
    }
}
